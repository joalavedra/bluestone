//! Faire External API v2 connector (wholesale marketplace).
//! Faire has no stock locations, so every variant's on-hand stock lives at one synthetic location.
use super::*;
use anyhow::{Context, bail};
use reqwest::Method;
use serde_json::{Value, json};
use std::collections::HashMap;

pub const LOCATION: &str = "faire";
const PAGE_SIZE: usize = 50;
const INVENTORY_BATCH: usize = 50;

pub struct Faire {
    base: String,
    app_credentials: String,
    token: String,
    http: reqwest::Client,
}

impl Faire {
    /// `credential` is `<base64 app credentials>:<oauth access token>`; base64 never contains `:`.
    pub fn new(base_url: &str, credential: String) -> Result<Self> {
        let (app, token) = credential
            .split_once(':')
            .context("Faire credential must be `<base64 app credentials>:<oauth access token>`")?;
        Ok(Self {
            base: base_url.trim_end_matches('/').to_owned(),
            app_credentials: app.to_owned(),
            token: token.to_owned(),
            http: reqwest::Client::new(),
        })
    }

    fn req(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}/external-api/v2{path}", self.base))
            .header("X-FAIRE-APP-CREDENTIALS", &self.app_credentials)
            .header("X-FAIRE-OAUTH-ACCESS-TOKEN", &self.token)
    }

    async fn send(&self, rb: reqwest::RequestBuilder, what: &str) -> Result<Value> {
        let resp = rb
            .send()
            .await
            .with_context(|| format!("faire {what} failed"))?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            bail!(
                "faire {what}: {status} {}",
                body.chars().take(300).collect::<String>()
            );
        }
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&body).with_context(|| format!("faire {what} returned invalid JSON"))
    }

    /// Follows `cursor` until a page comes back empty or without a new cursor.
    async fn paged(&self, path: &str, key: &str, extra: &[(&str, &str)]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut q: Vec<(&str, String)> = vec![("limit", PAGE_SIZE.to_string())];
            q.extend(extra.iter().map(|(k, v)| (*k, v.to_string())));
            if let Some(c) = &cursor {
                q.push(("cursor", c.clone()));
            }
            let v = self
                .send(
                    self.req(Method::GET, path).query(&q),
                    &format!("GET {path}"),
                )
                .await?;
            let page = v[key].as_array().cloned().unwrap_or_default();
            let empty = page.is_empty();
            out.extend(page);
            match v["cursor"].as_str().filter(|c| !c.is_empty()) {
                Some(c) if !empty && cursor.as_deref() != Some(c) => cursor = Some(c.to_owned()),
                _ => break,
            }
        }
        Ok(out)
    }

    /// On-hand quantity per variant id; untracked variants are left out.
    async fn on_hand(&self, variant_ids: &[String]) -> Result<HashMap<String, i64>> {
        let mut out = HashMap::new();
        for chunk in variant_ids.chunks(INVENTORY_BATCH) {
            let v = self
                .send(
                    self.req(Method::GET, "/product-inventory/by-product-variant-ids")
                        .query(&[("ids", chunk.join(","))]),
                    "GET inventory",
                )
                .await?;
            for (id, inv) in v["inventories"].as_object().into_iter().flatten() {
                let q = &inv["on_hand_quantity"];
                if q["type"] != "UNTRACKED"
                    && let Some(n) = q["quantity"].as_i64()
                {
                    out.insert(id.clone(), n);
                }
            }
        }
        Ok(out)
    }
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}

fn cents(v: &Value) -> Option<f64> {
    v.as_i64().map(|c| c as f64 / 100.0)
}

/// Bluestone tracks the wholesale price for Faire listings.
fn wholesale(variant: &Value) -> Option<f64> {
    variant["prices"]
        .get(0)
        .and_then(|p| cents(&p["wholesale_price"]["amount_minor"]))
        .or_else(|| cents(&variant["wholesale_price_cents"]))
}

pub fn status_from(lifecycle: &str) -> &'static str {
    match lifecycle {
        "PUBLISHED" => "active",
        "DELETED" => "archived",
        _ => "draft",
    }
}

/// `archived` unpublishes rather than deletes: Faire deletion is irreversible.
fn lifecycle_for(status: &str) -> Result<&'static str> {
    match status {
        "active" => Ok("PUBLISHED"),
        "draft" | "archived" => Ok("UNPUBLISHED"),
        other => bail!("unsupported status `{other}`"),
    }
}

#[async_trait]
impl Connector for Faire {
    async fn fetch_catalog(&self) -> Result<Catalog> {
        let products = self.paged("/products", "products", &[]).await?;
        let ids: Vec<String> = products
            .iter()
            .flat_map(|p| p["variants"].as_array().cloned().unwrap_or_default())
            .map(|v| s(&v["id"]))
            .collect();
        let stock = self.on_hand(&ids).await?;
        let mut listings = Vec::new();
        for p in &products {
            let lifecycle = p["lifecycle_state"].as_str().unwrap_or("DRAFT");
            if lifecycle == "DELETED" {
                continue;
            }
            let variants: Vec<&Value> = p["variants"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|v| v["lifecycle_state"] != "DELETED")
                .collect();
            let name = s(&p["name"]);
            for v in &variants {
                let id = s(&v["id"]);
                let vname = s(&v["name"]);
                listings.push(RemoteListing {
                    external_product_id: s(&p["id"]),
                    external_variant_id: id.clone(),
                    inventory_ref: None,
                    sku: Some(s(&v["sku"])).filter(|x| !x.is_empty()),
                    title: if variants.len() > 1 && !vname.is_empty() {
                        format!("{name} / {vname}")
                    } else {
                        name.clone()
                    },
                    price: wholesale(v),
                    status: Some(status_from(lifecycle).into()),
                    image_url: p["images"][0]["url"].as_str().map(Into::into),
                    stock: stock
                        .get(&id)
                        .map(|q| RemoteStock {
                            location_external_id: LOCATION.into(),
                            stock_ref: None,
                            quantity: *q,
                        })
                        .into_iter()
                        .collect(),
                });
            }
        }
        Ok(Catalog {
            locations: vec![RemoteLocation {
                external_id: LOCATION.into(),
                name: "Faire stock".into(),
            }],
            listings,
        })
    }

    async fn fetch_orders(&self, since: &str) -> Result<Vec<RemoteOrderLine>> {
        let orders = self
            .paged(
                "/orders",
                "orders",
                &[("created_at_min", since), ("excluded_states", "CANCELED")],
            )
            .await?;
        Ok(orders
            .iter()
            .filter(|o| o["state"] != "CANCELED")
            .flat_map(|o| {
                let order = s(&o["id"]);
                let at = s(&o["created_at"]);
                o["items"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|i| i["state"] != "CANCELED")
                    .map(move |i| RemoteOrderLine {
                        external_order_id: order.clone(),
                        external_line_id: s(&i["id"]),
                        external_product_id: s(&i["product_id"]),
                        external_variant_id: s(&i["variant_id"]),
                        sku: Some(s(&i["sku"])).filter(|x| !x.is_empty()),
                        quantity: i["quantity"].as_i64().unwrap_or(0),
                        unit_price: cents(&i["price"]["amount_minor"])
                            .or_else(|| cents(&i["price_cents"])),
                        ordered_at: at.clone(),
                    })
            })
            .collect())
    }

    /// Faire has no compare-and-set, so re-read on-hand first and refuse if it moved.
    async fn set_stock(
        &self,
        listing: &ListingRef,
        _location: &str,
        _stock_ref: Option<&str>,
        current: i64,
        quantity: i64,
    ) -> Result<()> {
        let id = &listing.external_variant_id;
        let now = self.on_hand(std::slice::from_ref(id)).await?;
        match now.get(id) {
            None => bail!("variant {id} has untracked or unknown inventory"),
            Some(q) if *q != current => {
                bail!("stock on Faire is now {q}, not {current}; re-propose")
            }
            _ => {}
        }
        self.send(
            self.req(Method::PATCH, "/product-inventory/by-product-variant-ids")
                .json(&json!({ "inventories": [{ "product_variant_id": id, "on_hand_quantity": quantity }] })),
            "PATCH inventory",
        )
        .await?;
        Ok(())
    }

    /// Updates the wholesale price in the variant's primary currency; retail and other currencies are kept.
    async fn set_price(&self, listing: &ListingRef, price: f64) -> Result<()> {
        let product = self
            .send(
                self.req(
                    Method::GET,
                    &format!("/products/{}", listing.external_product_id),
                ),
                "GET product",
            )
            .await?;
        let variant = product["variants"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|v| s(&v["id"]) == listing.external_variant_id)
            .with_context(|| format!("variant {} not found", listing.external_variant_id))?;
        let minor = (price * 100.0).round() as i64;
        let mut prices = variant["prices"].as_array().cloned().unwrap_or_default();
        if prices.is_empty() {
            bail!("variant has no price entries to update");
        }
        let currency = prices[0]["wholesale_price"]["currency"].clone();
        for p in &mut prices {
            if p["wholesale_price"]["currency"] == currency {
                p["wholesale_price"]["amount_minor"] = json!(minor);
            }
        }
        self.send(
            self.req(Method::PATCH, "/product-prices/by-product-variant-ids")
                .json(&json!({ "prices": [{ "product_variant_id": listing.external_variant_id, "prices": prices }] })),
            "PATCH prices",
        )
        .await?;
        Ok(())
    }

    async fn set_status(&self, listing: &ListingRef, status: &str) -> Result<()> {
        let lifecycle = lifecycle_for(status)?;
        self.send(
            self.req(
                Method::PATCH,
                &format!("/products/{}", listing.external_product_id),
            )
            .json(&json!({ "lifecycle_state": lifecycle })),
            "PATCH product",
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lifecycle_and_prices() {
        assert_eq!(status_from("PUBLISHED"), "active");
        assert_eq!(status_from("UNPUBLISHED"), "draft");
        assert_eq!(lifecycle_for("archived").unwrap(), "UNPUBLISHED");
        assert!(lifecycle_for("deleted").is_err());
        let v = json!({ "prices": [{ "wholesale_price": { "amount_minor": 1250, "currency": "EUR" } }] });
        assert_eq!(wholesale(&v), Some(12.5));
        assert_eq!(
            wholesale(&json!({ "wholesale_price_cents": 900 })),
            Some(9.0)
        );
        assert!(Faire::new("http://x", "no-colon".into()).is_err());
    }
}
