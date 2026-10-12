//! Local mock of the QuickBooks Online v3 and Holded invoicing v1 APIs, for dev and smoke tests.
//! `GET /_dump` returns everything that was created.

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Store {
    /// QuickBooks entities by type (Vendor, Customer, Item, Bill, Invoice).
    qbo: HashMap<String, Vec<Value>>,
    /// Holded contacts and documents by type (contacts, invoice, purchase).
    holded: HashMap<String, Vec<Value>>,
    next: u64,
}

struct Mock {
    store: Mutex<Store>,
    qbo_token: String,
    holded_key: String,
}

type Shared = Arc<Mock>;
type Reply = (StatusCode, Json<Value>);

fn err(code: StatusCode, msg: &str) -> Reply {
    (
        code,
        Json(
            json!({ "Fault": { "Error": [{ "Message": msg, "Detail": msg }] }, "status": 0, "info": msg }),
        ),
    )
}

fn qbo_auth(m: &Mock, h: &HeaderMap) -> Result<(), Reply> {
    let ok = h.get("authorization").and_then(|v| v.to_str().ok())
        == Some(&format!("Bearer {}", m.qbo_token));
    if ok {
        Ok(())
    } else {
        Err(err(StatusCode::UNAUTHORIZED, "AuthenticationFailed"))
    }
}

fn holded_auth(m: &Mock, h: &HeaderMap) -> Result<(), Reply> {
    if h.get("key").and_then(|v| v.to_str().ok()) == Some(m.holded_key.as_str()) {
        Ok(())
    } else {
        Err(err(StatusCode::UNAUTHORIZED, "invalid key"))
    }
}

/// Supports `select * from <Entity> where <Field> = '<value>'`.
async fn qbo_query(
    State(m): State<Shared>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Reply {
    if let Err(e) = qbo_auth(&m, &h) {
        return e;
    }
    let query = q.get("query").cloned().unwrap_or_default();
    let words: Vec<&str> = query.splitn(8, ' ').collect();
    let (Some(entity), Some(field)) = (words.get(3), words.get(5)) else {
        return err(StatusCode::BAD_REQUEST, "unsupported query");
    };
    let value = query
        .split_once(" = '")
        .map(|(_, v)| v.trim_end_matches('\'').replace("\\'", "'"))
        .unwrap_or_default();
    let store = m.store.lock().unwrap();
    let hits: Vec<Value> = store
        .qbo
        .get(*entity)
        .into_iter()
        .flatten()
        .filter(|e| e[*field].as_str() == Some(value.as_str()))
        .cloned()
        .collect();
    let body = if hits.is_empty() {
        json!({ "QueryResponse": {} })
    } else {
        json!({ "QueryResponse": { *entity: hits } })
    };
    (StatusCode::OK, Json(body))
}

async fn qbo_create(
    State(m): State<Shared>,
    h: HeaderMap,
    Path((_realm, entity)): Path<(String, String)>,
    Json(mut body): Json<Value>,
) -> Reply {
    if let Err(e) = qbo_auth(&m, &h) {
        return e;
    }
    let Some(name) = ["Vendor", "Customer", "Item", "Bill", "Invoice"]
        .into_iter()
        .find(|n| n.eq_ignore_ascii_case(&entity))
    else {
        return err(StatusCode::NOT_FOUND, "unknown entity");
    };
    let mut store = m.store.lock().unwrap();
    let exists = |store: &Store, kind: &str, id: &Value| {
        store
            .qbo
            .get(kind)
            .into_iter()
            .flatten()
            .any(|e| e["Id"] == *id)
    };
    match name {
        "Item" if body["IncomeAccountRef"]["value"].is_null() => {
            return err(StatusCode::BAD_REQUEST, "IncomeAccountRef required");
        }
        "Bill" | "Invoice" => {
            let (party, detail) = if name == "Bill" {
                ("VendorRef", "ItemBasedExpenseLineDetail")
            } else {
                ("CustomerRef", "SalesItemLineDetail")
            };
            let party_kind = if name == "Bill" { "Vendor" } else { "Customer" };
            if !exists(&store, party_kind, &body[party]["value"]) {
                return err(StatusCode::BAD_REQUEST, "Invalid party reference");
            }
            let lines = body["Line"].as_array().cloned().unwrap_or_default();
            if lines.is_empty()
                || lines
                    .iter()
                    .any(|l| !exists(&store, "Item", &l[detail]["ItemRef"]["value"]))
            {
                return err(StatusCode::BAD_REQUEST, "Invalid item reference");
            }
            let total: f64 = lines.iter().filter_map(|l| l["Amount"].as_f64()).sum();
            body["TotalAmt"] = json!((total * 100.0).round() / 100.0);
        }
        _ => {}
    }
    store.next += 1;
    body["Id"] = json!(store.next.to_string());
    store.qbo.entry(name.into()).or_default().push(body.clone());
    (StatusCode::OK, Json(json!({ name: body })))
}

async fn holded_contacts(State(m): State<Shared>, h: HeaderMap) -> Reply {
    if let Err(e) = holded_auth(&m, &h) {
        return e;
    }
    let store = m.store.lock().unwrap();
    (
        StatusCode::OK,
        Json(json!(
            store.holded.get("contacts").cloned().unwrap_or_default()
        )),
    )
}

async fn holded_create_contact(
    State(m): State<Shared>,
    h: HeaderMap,
    Json(mut body): Json<Value>,
) -> Reply {
    if let Err(e) = holded_auth(&m, &h) {
        return e;
    }
    let mut store = m.store.lock().unwrap();
    store.next += 1;
    let id = format!("hc{:022}", store.next);
    body["id"] = json!(id);
    store
        .holded
        .entry("contacts".into())
        .or_default()
        .push(body);
    (
        StatusCode::CREATED,
        Json(json!({ "status": 1, "info": "Created", "id": id })),
    )
}

async fn holded_create_document(
    State(m): State<Shared>,
    h: HeaderMap,
    Path(doc_type): Path<String>,
    Json(mut body): Json<Value>,
) -> Reply {
    if let Err(e) = holded_auth(&m, &h) {
        return e;
    }
    if !["invoice", "purchase", "salesorder", "purchaseorder"].contains(&doc_type.as_str()) {
        return err(StatusCode::BAD_REQUEST, "invalid docType");
    }
    let mut store = m.store.lock().unwrap();
    let contact_ok = store
        .holded
        .get("contacts")
        .into_iter()
        .flatten()
        .any(|c| c["id"] == body["contactId"]);
    if !contact_ok
        || body["items"].as_array().is_none_or(|i| i.is_empty())
        || !body["date"].is_i64()
    {
        return err(
            StatusCode::BAD_REQUEST,
            "contactId, date and items are required",
        );
    }
    store.next += 1;
    let id = format!("hd{:022}", store.next);
    body["id"] = json!(id);
    body["docNumber"] = body["invoiceNum"].clone();
    body["contact"] = body["contactId"].clone();
    store.holded.entry(doc_type).or_default().push(body);
    (
        StatusCode::CREATED,
        Json(json!({ "status": 1, "info": "Created", "id": id })),
    )
}

async fn holded_documents(
    State(m): State<Shared>,
    h: HeaderMap,
    Path(doc_type): Path<String>,
) -> Reply {
    if let Err(e) = holded_auth(&m, &h) {
        return e;
    }
    let store = m.store.lock().unwrap();
    (
        StatusCode::OK,
        Json(json!(
            store.holded.get(&doc_type).cloned().unwrap_or_default()
        )),
    )
}

async fn qbo_company(
    State(m): State<Shared>,
    h: HeaderMap,
    Path((realm, id)): Path<(String, String)>,
) -> Reply {
    if let Err(e) = qbo_auth(&m, &h) {
        return e;
    }
    if realm != id {
        return err(StatusCode::BAD_REQUEST, "company id mismatch");
    }
    (
        StatusCode::OK,
        Json(
            json!({ "CompanyInfo": { "Id": realm, "CompanyName": format!("Mock Company {realm}") } }),
        ),
    )
}

async fn dump(State(m): State<Shared>) -> Json<Value> {
    let store = m.store.lock().unwrap();
    Json(json!({ "quickbooks": store.qbo, "holded": store.holded }))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = std::env::var("MOCK_ACCOUNTING_ADDR").unwrap_or_else(|_| "127.0.0.1:8790".into());
    let m: Shared = Arc::new(Mock {
        store: Mutex::default(),
        qbo_token: std::env::var("MOCK_QUICKBOOKS_TOKEN").unwrap_or_else(|_| "qbo_dev_mock".into()),
        holded_key: std::env::var("MOCK_HOLDED_KEY").unwrap_or_else(|_| "holded_dev_mock".into()),
    });
    let app = Router::new()
        .route("/v3/company/{realm}/query", get(qbo_query))
        .route("/v3/company/{realm}/{entity}", post(qbo_create))
        .route("/v3/company/{realm}/companyinfo/{id}", get(qbo_company))
        .route(
            "/api/invoicing/v1/contacts",
            get(holded_contacts).post(holded_create_contact),
        )
        .route(
            "/api/invoicing/v1/documents/{doc_type}",
            get(holded_documents).post(holded_create_document),
        )
        .route("/_dump", get(dump))
        .with_state(m);
    eprintln!("mock QuickBooks + Holded on http://{addr}");
    axum::serve(tokio::net::TcpListener::bind(&addr).await?, app).await?;
    Ok(())
}
