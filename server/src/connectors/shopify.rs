//! Shopify Admin GraphQL connector.
use super::*;
use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};

pub const API_VERSION: &str = "2025-07";

pub struct Shopify {
    endpoint: String,
    token: String,
    http: reqwest::Client,
}

impl Shopify {
    pub fn new(base_url: &str, token: String) -> Self {
        let base = base_url.trim_end_matches('/');
        Self {
            endpoint: format!("{base}/admin/api/{API_VERSION}/graphql.json"),
            token,
            http: reqwest::Client::new(),
        }
    }

    async fn gql(&self, query: &str, variables: Value) -> Result<Value> {
        let resp: Value = self
            .http
            .post(&self.endpoint)
            .header("X-Shopify-Access-Token", &self.token)
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .context("shopify request failed")?
            .error_for_status()?
            .json()
            .await?;
        if let Some(errors) = resp.get("errors") {
            bail!("shopify graphql errors: {errors}");
        }
        resp.get("data")
            .cloned()
            .ok_or_else(|| anyhow!("shopify response missing data"))
    }

    fn check_user_errors(data: &Value, field: &str) -> Result<()> {
        let errs = &data[field]["userErrors"];
        if errs.as_array().is_some_and(|a| !a.is_empty()) {
            bail!("shopify {field} failed: {errs}");
        }
        Ok(())
    }
}

const LOCATIONS: &str = "query { locations(first: 50) { nodes { id name } } }";

const PRODUCTS: &str = r#"query Products($after: String) {
  products(first: 50, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      id title status
      featuredImage { url }
      variants(first: 100) {
        nodes {
          id sku title price
          inventoryItem {
            id
            inventoryLevels(first: 20) {
              nodes { location { id } quantities(names: ["available"]) { name quantity } }
            }
          }
        }
      }
    }
  }
}"#;

const ORDERS: &str = r#"query Orders($after: String, $query: String) {
  orders(first: 100, after: $after, query: $query) {
    pageInfo { hasNextPage endCursor }
    nodes {
      id createdAt
      lineItems(first: 100) {
        nodes {
          id quantity sku
          variant { id product { id } }
          originalUnitPriceSet { shopMoney { amount } }
        }
      }
    }
  }
}"#;

fn money(v: &Value) -> Option<f64> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

#[async_trait]
impl Connector for Shopify {
    async fn fetch_catalog(&self) -> Result<Catalog> {
        let locs = self.gql(LOCATIONS, json!({})).await?;
        let locations = locs["locations"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| RemoteLocation {
                external_id: l["id"].as_str().unwrap_or_default().to_owned(),
                name: l["name"].as_str().unwrap_or_default().to_owned(),
            })
            .collect();

        let mut listings = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let data = self.gql(PRODUCTS, json!({ "after": after })).await?;
            let page = &data["products"];
            for p in page["nodes"].as_array().into_iter().flatten() {
                let product_title = p["title"].as_str().unwrap_or_default();
                for v in p["variants"]["nodes"].as_array().into_iter().flatten() {
                    let variant_title = v["title"].as_str().unwrap_or_default();
                    let title = if variant_title.is_empty() || variant_title == "Default Title" {
                        product_title.to_owned()
                    } else {
                        format!("{product_title} / {variant_title}")
                    };
                    let stock = v["inventoryItem"]["inventoryLevels"]["nodes"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|lvl| RemoteStock {
                            location_external_id: lvl["location"]["id"]
                                .as_str()
                                .unwrap_or_default()
                                .to_owned(),
                            stock_ref: None,
                            quantity: lvl["quantities"]
                                .as_array()
                                .and_then(|q| q.iter().find(|q| q["name"] == "available"))
                                .and_then(|q| q["quantity"].as_i64())
                                .unwrap_or(0),
                        })
                        .collect();
                    listings.push(RemoteListing {
                        external_product_id: p["id"].as_str().unwrap_or_default().to_owned(),
                        external_variant_id: v["id"].as_str().unwrap_or_default().to_owned(),
                        inventory_ref: opt_str(&v["inventoryItem"]["id"]),
                        sku: opt_str(&v["sku"]),
                        title,
                        price: money(&v["price"]),
                        status: p["status"].as_str().map(str::to_lowercase),
                        image_url: opt_str(&p["featuredImage"]["url"]),
                        stock,
                    });
                }
            }
            if page["pageInfo"]["hasNextPage"].as_bool() == Some(true) {
                after = opt_str(&page["pageInfo"]["endCursor"]);
            } else {
                break;
            }
        }
        Ok(Catalog {
            locations,
            listings,
        })
    }

    async fn fetch_orders(&self, since: &str) -> Result<Vec<RemoteOrderLine>> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        let query = format!("created_at:>='{since}'");
        loop {
            let data = self
                .gql(ORDERS, json!({ "after": after, "query": query }))
                .await?;
            let page = &data["orders"];
            for o in page["nodes"].as_array().into_iter().flatten() {
                for li in o["lineItems"]["nodes"].as_array().into_iter().flatten() {
                    out.push(RemoteOrderLine {
                        external_order_id: o["id"].as_str().unwrap_or_default().to_owned(),
                        external_line_id: li["id"].as_str().unwrap_or_default().to_owned(),
                        external_product_id: li["variant"]["product"]["id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        external_variant_id: li["variant"]["id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                        sku: opt_str(&li["sku"]),
                        quantity: li["quantity"].as_i64().unwrap_or(0),
                        unit_price: money(&li["originalUnitPriceSet"]["shopMoney"]["amount"]),
                        ordered_at: o["createdAt"].as_str().unwrap_or_default().to_owned(),
                    });
                }
            }
            if page["pageInfo"]["hasNextPage"].as_bool() == Some(true) {
                after = opt_str(&page["pageInfo"]["endCursor"]);
            } else {
                break;
            }
        }
        Ok(out)
    }

    async fn set_stock(
        &self,
        listing: &ListingRef,
        location_external_id: &str,
        _stock_ref: Option<&str>,
        current: i64,
        quantity: i64,
    ) -> Result<()> {
        let inventory_item = listing
            .inventory_ref
            .as_deref()
            .ok_or_else(|| anyhow!("listing has no Shopify inventory item"))?;
        let q = r#"mutation Set($input: InventorySetQuantitiesInput!) {
  inventorySetQuantities(input: $input) { userErrors { field message } }
}"#;
        let input = json!({
            "name": "available",
            "reason": "correction",
            "referenceDocumentUri": "bluestone://proposal",
            "quantities": [{
                "inventoryItemId": inventory_item,
                "locationId": location_external_id,
                "quantity": quantity,
                "compareQuantity": current,
            }],
        });
        let data = self.gql(q, json!({ "input": input })).await?;
        Self::check_user_errors(&data, "inventorySetQuantities")
    }

    async fn set_price(&self, listing: &ListingRef, price: f64) -> Result<()> {
        let q = r#"mutation Price($productId: ID!, $variants: [ProductVariantsBulkInput!]!) {
  productVariantsBulkUpdate(productId: $productId, variants: $variants) { userErrors { field message } }
}"#;
        let vars = json!({
            "productId": listing.external_product_id,
            "variants": [{ "id": listing.external_variant_id, "price": format!("{price:.2}") }],
        });
        let data = self.gql(q, vars).await?;
        Self::check_user_errors(&data, "productVariantsBulkUpdate")
    }

    async fn set_status(&self, listing: &ListingRef, status: &str) -> Result<()> {
        let q = r#"mutation Status($product: ProductUpdateInput!) {
  productUpdate(product: $product) { userErrors { field message } }
}"#;
        let vars = json!({ "product": { "id": listing.external_product_id, "status": status.to_uppercase() } });
        let data = self.gql(q, vars).await?;
        Self::check_user_errors(&data, "productUpdate")
    }
}
