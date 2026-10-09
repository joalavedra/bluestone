//! Minimal in-memory mock of the Shopify Admin GraphQL API, seeded with a demo coffee brand.
#![allow(clippy::collapsible_if)]
//! Supports exactly the queries/mutations Bluestone's connector uses.
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

struct Variant {
    id: String,
    inventory_item: String,
    sku: String,
    title: String,
    price: f64,
    stock: Vec<i64>,
}

struct Product {
    id: String,
    title: String,
    status: String,
    image: String,
    variants: Vec<Variant>,
}

struct Line {
    id: String,
    product: String,
    variant: String,
    sku: String,
    qty: i64,
    price: f64,
}

struct Order {
    id: String,
    created_at: String,
    lines: Vec<Line>,
}

struct Store {
    locations: Vec<(String, String)>,
    products: Vec<Product>,
    orders: Vec<Order>,
}

type Shared = Arc<Mutex<Store>>;

/// (title, image keyword, [(variant title, sku, price, [stock per location], daily demand)])
type Seed = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str, f64, [i64; 2], f64)],
);

const SEED: &[Seed] = &[
    (
        "Ethiopia Yirgacheffe",
        "coffee-beans",
        &[
            ("250g", "NW-ETH-250", 14.5, [42, 8], 2.4),
            ("1kg", "NW-ETH-1K", 48.0, [6, 0], 0.6),
        ],
    ),
    (
        "Colombia Huila",
        "coffee",
        &[
            ("250g", "NW-COL-250", 13.0, [120, 30], 3.1),
            ("1kg", "NW-COL-1K", 44.0, [18, 2], 0.5),
        ],
    ),
    (
        "House Espresso Blend",
        "espresso",
        &[
            ("250g", "NW-ESP-250", 12.0, [9, 3], 4.2),
            ("1kg", "NW-ESP-1K", 39.0, [25, 4], 0.9),
        ],
    ),
    (
        "Decaf Swiss Water",
        "coffee-cup",
        &[("Default Title", "NW-DEC-250", 13.5, [64, 12], 0.4)],
    ),
    (
        "Ceramic Pour-Over Dripper",
        "pour-over",
        &[
            ("White", "NW-DRP-WHT", 29.0, [0, 0], 0.5),
            ("Black", "NW-DRP-BLK", 29.0, [14, 3], 0.3),
        ],
    ),
    (
        "Paper Filters (100)",
        "filter",
        &[("Default Title", "NW-FLT-100", 6.5, [210, 40], 2.8)],
    ),
    (
        "Gooseneck Kettle",
        "kettle",
        &[("Default Title", "NW-KTL-01", 59.0, [7, 1], 0.25)],
    ),
    (
        "Hand Grinder",
        "grinder",
        &[("Default Title", "NW-GRD-01", 89.0, [3, 0], 0.2)],
    ),
    (
        "Cold Brew Bottle",
        "cold-brew",
        &[("Default Title", "NW-CBB-01", 24.0, [48, 6], 0.1)],
    ),
    (
        "Gift Card",
        "gift",
        &[("Default Title", "", 25.0, [0, 0], 0.15)],
    ),
];

fn seed() -> Store {
    let locations = vec![
        (
            "gid://shopify/Location/1001".to_owned(),
            "Barcelona warehouse".to_owned(),
        ),
        (
            "gid://shopify/Location/1002".to_owned(),
            "Madrid store".to_owned(),
        ),
    ];
    let mut products = Vec::new();
    let mut demand = Vec::new();
    let mut vid = 5000;
    for (pi, (title, img, variants)) in SEED.iter().enumerate() {
        let pid = format!("gid://shopify/Product/{}", 100 + pi);
        let mut vs = Vec::new();
        for (vt, sku, price, stock, daily) in variants.iter() {
            vid += 1;
            vs.push(Variant {
                id: format!("gid://shopify/ProductVariant/{vid}"),
                inventory_item: format!("gid://shopify/InventoryItem/{vid}"),
                sku: (*sku).into(),
                title: (*vt).into(),
                price: *price,
                stock: stock.to_vec(),
            });
            demand.push((
                pid.clone(),
                format!("gid://shopify/ProductVariant/{vid}"),
                (*sku).to_owned(),
                *price,
                *daily,
            ));
        }
        products.push(Product {
            id: pid,
            title: (*title).into(),
            status: if *title == "Cold Brew Bottle" {
                "DRAFT".into()
            } else {
                "ACTIVE".into()
            },
            image: format!("https://placehold.co/96x96/eef3fc/2170e4?text={}", img),
            variants: vs,
        });
    }
    // Deterministic pseudo-random orders over the last 60 days.
    let mut rng: u64 = 0x2170e4;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng % 10_000) as f64 / 10_000.0
    };
    let mut orders = Vec::new();
    let mut line_id = 90_000;
    let now = Utc::now();
    for day in (0..60).rev() {
        for slot in 0..6 {
            let mut lines = Vec::new();
            for (pid, vid, sku, price, daily) in &demand {
                if next() < daily / 6.0 {
                    line_id += 1;
                    lines.push(Line {
                        id: format!("gid://shopify/LineItem/{line_id}"),
                        product: pid.clone(),
                        variant: vid.clone(),
                        sku: sku.clone(),
                        qty: 1 + (next() * 1.6) as i64,
                        price: *price,
                    });
                }
            }
            if !lines.is_empty() {
                let at = now - Duration::days(day) - Duration::hours(slot * 3 + 1);
                orders.push(Order {
                    id: format!("gid://shopify/Order/{}", 70_000 + orders.len()),
                    created_at: at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                    lines,
                });
            }
        }
    }
    Store {
        locations,
        products,
        orders,
    }
}

fn page(nodes: Vec<Value>) -> Value {
    json!({ "nodes": nodes, "pageInfo": { "hasNextPage": false, "endCursor": null } })
}

fn handle(store: &mut Store, query: &str, vars: &Value) -> Value {
    if query.contains("inventorySetQuantities") {
        let mut errors = Vec::new();
        for q in vars["input"]["quantities"].as_array().into_iter().flatten() {
            let loc = store
                .locations
                .iter()
                .position(|(id, _)| q["locationId"] == *id.as_str());
            let variant = store
                .products
                .iter_mut()
                .flat_map(|p| p.variants.iter_mut())
                .find(|v| q["inventoryItemId"] == *v.inventory_item.as_str());
            match (loc, variant) {
                (Some(l), Some(v)) => {
                    if let Some(cmp) = q["compareQuantity"].as_i64() {
                        if v.stock[l] != cmp {
                            errors.push(json!({ "field": ["input", "quantities", "0", "compareQuantity"], "message": format!("The compareQuantity value does not match the current quantity ({})", v.stock[l]) }));
                            continue;
                        }
                    }
                    v.stock[l] = q["quantity"].as_i64().unwrap_or(0);
                }
                _ => errors.push(json!({ "field": ["input"], "message": "inventory item or location not found" })),
            }
        }
        return json!({ "inventorySetQuantities": { "userErrors": errors } });
    }
    if query.contains("productVariantsBulkUpdate") {
        let mut errors = Vec::new();
        for upd in vars["variants"].as_array().into_iter().flatten() {
            let found = store
                .products
                .iter_mut()
                .flat_map(|p| p.variants.iter_mut())
                .find(|v| upd["id"] == *v.id.as_str());
            match (
                found,
                upd["price"].as_str().and_then(|p| p.parse::<f64>().ok()),
            ) {
                (Some(v), Some(price)) => v.price = price,
                _ => errors.push(
                    json!({ "field": ["variants"], "message": "variant not found or bad price" }),
                ),
            }
        }
        return json!({ "productVariantsBulkUpdate": { "userErrors": errors } });
    }
    if query.contains("productUpdate") {
        let input = &vars["product"];
        let errors = match store
            .products
            .iter_mut()
            .find(|p| input["id"] == *p.id.as_str())
        {
            Some(p) => {
                p.status = input["status"].as_str().unwrap_or("ACTIVE").to_owned();
                vec![]
            }
            None => vec![json!({ "field": ["id"], "message": "product not found" })],
        };
        return json!({ "productUpdate": { "userErrors": errors } });
    }
    if query.contains("locations(") {
        let nodes: Vec<Value> = store
            .locations
            .iter()
            .map(|(id, name)| json!({ "id": id, "name": name }))
            .collect();
        return json!({ "locations": { "nodes": nodes } });
    }
    if query.contains("products(") {
        let locs = &store.locations;
        let nodes = store
            .products
            .iter()
            .map(|p| {
                let variants: Vec<Value> = p
                    .variants
                    .iter()
                    .map(|v| {
                        let levels: Vec<Value> = locs
                            .iter()
                            .zip(&v.stock)
                            .map(|((lid, _), q)| json!({ "location": { "id": lid }, "quantities": [{ "name": "available", "quantity": q }] }))
                            .collect();
                        json!({
                            "id": v.id, "sku": v.sku, "title": v.title, "price": format!("{:.2}", v.price),
                            "inventoryItem": { "id": v.inventory_item, "inventoryLevels": { "nodes": levels } }
                        })
                    })
                    .collect();
                json!({ "id": p.id, "title": p.title, "status": p.status, "featuredImage": { "url": p.image }, "variants": { "nodes": variants } })
            })
            .collect();
        return json!({ "products": page(nodes) });
    }
    if query.contains("orders(") {
        let since = vars["query"]
            .as_str()
            .and_then(|q| q.split('\'').nth(1))
            .unwrap_or("")
            .to_owned();
        let nodes = store
            .orders
            .iter()
            .filter(|o| o.created_at.as_str() >= since.as_str())
            .map(|o| {
                let lines: Vec<Value> = o
                    .lines
                    .iter()
                    .map(|l| json!({
                        "id": l.id, "quantity": l.qty, "sku": l.sku,
                        "variant": { "id": l.variant, "product": { "id": l.product } },
                        "originalUnitPriceSet": { "shopMoney": { "amount": format!("{:.2}", l.price) } }
                    }))
                    .collect();
                json!({ "id": o.id, "createdAt": o.created_at, "lineItems": { "nodes": lines } })
            })
            .collect();
        return json!({ "orders": page(nodes) });
    }
    Value::Null
}

async fn graphql(
    State(store): State<(Shared, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let (store, token) = store;
    if headers
        .get("X-Shopify-Access-Token")
        .and_then(|h| h.to_str().ok())
        != Some(token.as_str())
    {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "errors": "[API] Invalid API key or access token" })),
        );
    }
    let query = body["query"].as_str().unwrap_or_default();
    let data = handle(&mut store.lock().unwrap(), query, &body["variables"]);
    if data.is_null() {
        return (
            StatusCode::OK,
            Json(json!({ "errors": [{ "message": "mock-shopify: unsupported query" }] })),
        );
    }
    (StatusCode::OK, Json(json!({ "data": data })))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = std::env::var("MOCK_SHOPIFY_ADDR").unwrap_or_else(|_| "127.0.0.1:8788".into());
    let token = std::env::var("MOCK_SHOPIFY_TOKEN").unwrap_or_else(|_| "shpat_dev_mock".into());
    let store: Shared = Arc::new(Mutex::new(seed()));
    let app = Router::new()
        .route("/admin/api/{version}/graphql.json", post(graphql))
        .with_state((store, token));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("mock-shopify listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
