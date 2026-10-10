//! In-memory Faire External API v2 for local development and the smoke test.
//! Wholesale listings of the same Northwind catalog as `mock_shopify`, matched by SKU.
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const PAGE: usize = 2;

struct Store {
    products: Vec<Value>,
    on_hand: HashMap<String, i64>,
    orders: Vec<Value>,
}

type Shared = Arc<(Mutex<Store>, String, String)>;

fn price(cents: i64) -> Value {
    json!([{ "geo_constraint": { "country": "ES" },
             "wholesale_price": { "amount_minor": cents, "currency": "EUR" },
             "retail_price": { "amount_minor": cents * 2, "currency": "EUR" } }])
}

fn variant(pid: &str, id: &str, sku: &str, name: &str, cents: i64) -> Value {
    json!({ "id": id, "product_id": pid, "sku": sku, "name": name, "lifecycle_state": "PUBLISHED", "prices": price(cents) })
}

fn seed() -> Store {
    let products = vec![
        json!({ "id": "p_esp", "name": "Northwind Espresso Beans 250g", "lifecycle_state": "PUBLISHED",
                "images": [{ "url": "https://picsum.photos/seed/nw-esp/200" }],
                "variants": [variant("p_esp", "v_esp", "NW-ESP-250", "", 650)] }),
        json!({ "id": "p_gift", "name": "Northwind Gift Box", "lifecycle_state": "PUBLISHED",
                "images": [], "variants": [
                    variant("p_gift", "v_gift_s", "NW-GIFT-S", "Small", 1800),
                    variant("p_gift", "v_gift_l", "NW-GIFT-L", "Large", 3200)] }),
        json!({ "id": "p_old", "name": "Discontinued sampler", "lifecycle_state": "DELETED",
                "images": [], "variants": [variant("p_old", "v_old", "NW-OLD", "", 100)] }),
    ];
    let on_hand = HashMap::from([
        ("v_esp".to_owned(), 24),
        ("v_gift_s".to_owned(), 6),
        ("v_gift_l".to_owned(), 0),
        ("v_old".to_owned(), 3),
    ]);
    let at = |d: i64| {
        (Utc::now() - Duration::days(d))
            .format("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    };
    let item = |id: &str, pid: &str, vid: &str, sku: &str, q: i64, cents: i64, state: &str| {
        json!({ "id": id, "product_id": pid, "variant_id": vid, "sku": sku, "quantity": q, "state": state,
                "price": { "amount_minor": cents, "currency": "EUR" } })
    };
    let orders = vec![
        json!({ "id": "bo_1", "state": "DELIVERED", "created_at": at(12), "items": [
            item("oi_1", "p_esp", "v_esp", "NW-ESP-250", 12, 650, "DELIVERED"),
            item("oi_2", "p_gift", "v_gift_s", "NW-GIFT-S", 4, 1800, "DELIVERED")] }),
        json!({ "id": "bo_2", "state": "PROCESSING", "created_at": at(3), "items": [
            item("oi_3", "p_esp", "v_esp", "NW-ESP-250", 6, 650, "PROCESSING"),
            item("oi_4", "p_gift", "v_gift_l", "NW-GIFT-L", 2, 3200, "CANCELED")] }),
        json!({ "id": "bo_3", "state": "CANCELED", "created_at": at(1), "items": [
            item("oi_5", "p_esp", "v_esp", "NW-ESP-250", 50, 650, "CANCELED")] }),
    ];
    Store {
        products,
        on_hand,
        orders,
    }
}

fn authed(s: &Shared, h: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    let ok = h
        .get("X-FAIRE-APP-CREDENTIALS")
        .and_then(|v| v.to_str().ok())
        == Some(s.1.as_str())
        && h.get("X-FAIRE-OAUTH-ACCESS-TOKEN")
            .and_then(|v| v.to_str().ok())
            == Some(s.2.as_str());
    if ok {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "message": "invalid credentials" })),
        ))
    }
}

type Res = Result<Json<Value>, (StatusCode, Json<Value>)>;

fn page(all: Vec<Value>, key: &str, q: &HashMap<String, String>) -> Value {
    let start: usize = q.get("cursor").and_then(|c| c.parse().ok()).unwrap_or(0);
    let end = (start + PAGE).min(all.len());
    let cursor = if end < all.len() {
        end.to_string()
    } else {
        String::new()
    };
    json!({ key: all[start.min(end)..end], "cursor": cursor })
}

async fn products(
    State(s): State<Shared>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Res {
    authed(&s, &h)?;
    let st = s.0.lock().unwrap();
    let live: Vec<Value> = st
        .products
        .iter()
        .filter(|p| p["lifecycle_state"] != "DELETED")
        .cloned()
        .collect();
    Ok(Json(page(live, "products", &q)))
}

async fn product(State(s): State<Shared>, h: HeaderMap, Path(id): Path<String>) -> Res {
    authed(&s, &h)?;
    let st = s.0.lock().unwrap();
    st.products
        .iter()
        .find(|p| p["id"] == *id.as_str())
        .cloned()
        .map(Json)
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "not found" })),
        ))
}

async fn patch_product(
    State(s): State<Shared>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(b): Json<Value>,
) -> Res {
    authed(&s, &h)?;
    let mut st = s.0.lock().unwrap();
    let p = st
        .products
        .iter_mut()
        .find(|p| p["id"] == *id.as_str())
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "not found" })),
        ))?;
    if let Some(l) = b.get("lifecycle_state") {
        p["lifecycle_state"] = l.clone();
    }
    Ok(Json(p.clone()))
}

fn inventory(st: &Store, id: &str) -> Value {
    let q = st.on_hand.get(id).copied().unwrap_or(0);
    let qty = |n: i64| json!({ "type": "QUANTITY", "quantity": n });
    json!({ "on_hand_quantity": qty(q), "committed_quantity": qty(0), "available_quantity": qty(q) })
}

async fn get_inventory(
    State(s): State<Shared>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Res {
    authed(&s, &h)?;
    let st = s.0.lock().unwrap();
    let inv: serde_json::Map<String, Value> = q
        .get("ids")
        .map(String::as_str)
        .unwrap_or("")
        .split(',')
        .filter(|id| st.on_hand.contains_key(*id))
        .map(|id| (id.to_owned(), inventory(&st, id)))
        .collect();
    Ok(Json(json!({ "inventories": inv })))
}

async fn patch_inventory(State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>) -> Res {
    authed(&s, &h)?;
    let mut st = s.0.lock().unwrap();
    let mut out = serde_json::Map::new();
    for i in b["inventories"].as_array().into_iter().flatten() {
        let id = i["product_variant_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if !st.on_hand.contains_key(&id) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({ "message": format!("unknown variant {id}") })),
            ));
        }
        st.on_hand
            .insert(id.clone(), i["on_hand_quantity"].as_i64().unwrap_or(0));
        out.insert(id.clone(), inventory(&st, &id));
    }
    Ok(Json(json!({ "inventories": out })))
}

async fn patch_prices(State(s): State<Shared>, h: HeaderMap, Json(b): Json<Value>) -> Res {
    authed(&s, &h)?;
    let mut st = s.0.lock().unwrap();
    for p in b["prices"].as_array().into_iter().flatten() {
        let v = st
            .products
            .iter_mut()
            .flat_map(|x| x["variants"].as_array_mut().into_iter().flatten())
            .find(|v| v["id"] == p["product_variant_id"])
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(json!({ "message": "unknown variant" })),
            ))?;
        v["prices"] = p["prices"].clone();
    }
    Ok(Json(json!({ "results": {} })))
}

async fn orders(
    State(s): State<Shared>,
    h: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Res {
    authed(&s, &h)?;
    let st = s.0.lock().unwrap();
    let min = q.get("created_at_min").cloned().unwrap_or_default();
    let excluded: Vec<&str> = q
        .get("excluded_states")
        .map(|x| x.split(',').collect())
        .unwrap_or_default();
    let all: Vec<Value> = st
        .orders
        .iter()
        .filter(|o| o["created_at"].as_str().unwrap_or_default() >= min.as_str())
        .filter(|o| !excluded.contains(&o["state"].as_str().unwrap_or_default()))
        .cloned()
        .collect();
    Ok(Json(page(all, "orders", &q)))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = std::env::var("MOCK_FAIRE_ADDR").unwrap_or_else(|_| "127.0.0.1:8789".into());
    let app_creds =
        std::env::var("MOCK_FAIRE_APP_CREDENTIALS").unwrap_or_else(|_| "ZGV2OmRldg==".into());
    let token = std::env::var("MOCK_FAIRE_TOKEN").unwrap_or_else(|_| "faire_dev_mock".into());
    let state: Shared = Arc::new((Mutex::new(seed()), app_creds, token));
    let app = Router::new()
        .route("/external-api/v2/products", get(products))
        .route(
            "/external-api/v2/products/{id}",
            get(product).patch(patch_product),
        )
        .route(
            "/external-api/v2/product-inventory/by-product-variant-ids",
            get(get_inventory).patch(patch_inventory),
        )
        .route(
            "/external-api/v2/product-prices/by-product-variant-ids",
            axum::routing::patch(patch_prices),
        )
        .route("/external-api/v2/orders", get(orders))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("mock Faire on http://{addr} (credential ZGV2OmRldg==:faire_dev_mock)");
    axum::serve(listener, app).await?;
    Ok(())
}
