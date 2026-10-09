//! PrestaShop legacy WebService connector (works on 1.7 – 9).
//! Reads use `output_format=JSON`; writes GET the resource XML, patch fields and PUT it back.
use super::*;
use anyhow::{Context, bail};
use serde_json::Value;
use std::collections::HashMap;

pub const DEFAULT_LOCATION: &str = "default";

pub struct PrestaShop {
    base: String,
    key: String,
    http: reqwest::Client,
}

impl PrestaShop {
    pub fn new(base_url: &str, key: String) -> Self {
        Self {
            base: base_url.trim_end_matches('/').to_owned(),
            key,
            http: reqwest::Client::new(),
        }
    }

    async fn get_json(&self, resource: &str, display: &str) -> Result<Vec<Value>> {
        let url = format!("{}/api/{resource}", self.base);
        let resp: Value = self
            .http
            .get(&url)
            .basic_auth(&self.key, Some(""))
            .query(&[
                ("display", display),
                ("output_format", "JSON"),
                ("language", "1"),
            ])
            .send()
            .await
            .with_context(|| format!("prestashop GET {resource} failed"))?
            .error_for_status()?
            .json()
            .await
            // Empty collections come back as `[]` rather than an object.
            .unwrap_or(Value::Null);
        Ok(resp
            .get(resource)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    async fn get_xml(&self, resource: &str, id: &str) -> Result<String> {
        let url = format!("{}/api/{resource}/{id}", self.base);
        Ok(self
            .http
            .get(&url)
            .basic_auth(&self.key, Some(""))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?)
    }

    async fn put_xml(&self, resource: &str, id: &str, body: String) -> Result<()> {
        let url = format!("{}/api/{resource}/{id}", self.base);
        let resp = self
            .http
            .put(&url)
            .basic_auth(&self.key, Some(""))
            .header("Content-Type", "application/xml")
            .body(body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            bail!(
                "prestashop PUT {resource}/{id} returned {status}: {}",
                truncate(&text, 400)
            );
        }
        Ok(())
    }

    async fn patch(&self, resource: &str, id: &str, fields: &[(&str, String)]) -> Result<()> {
        let mut xml = self.get_xml(resource, id).await?;
        for tag in READ_ONLY_FIELDS {
            xml = remove_tag(&xml, tag);
        }
        for (tag, value) in fields {
            xml = replace_tag(&xml, tag, value)
                .with_context(|| format!("field `{tag}` not found in {resource}/{id}"))?;
        }
        self.put_xml(resource, id, xml).await
    }
}

/// Fields the WebService returns but rejects on PUT.
const READ_ONLY_FIELDS: &[&str] = &[
    "manufacturer_name",
    "quantity",
    "position_in_category",
    // PS 8 fails the whole PUT with error 85 when the (usually empty) bundle association is echoed back.
    "product_bundle",
];

fn truncate(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Replace the text of the first `<tag ...>...</tag>` (or `<tag/>`) with a CDATA value.
pub fn replace_tag(xml: &str, tag: &str, value: &str) -> Option<String> {
    let (start, end) = find_tag(xml, tag)?;
    let open_end = xml[start..].find('>')? + start;
    let attrs = xml[start + 1 + tag.len()..open_end].trim_end_matches('/');
    Some(format!(
        "{}<{tag}{attrs}><![CDATA[{value}]]></{tag}>{}",
        &xml[..start],
        &xml[end..]
    ))
}

pub fn remove_tag(xml: &str, tag: &str) -> String {
    match find_tag(xml, tag) {
        Some((s, e)) => format!("{}{}", &xml[..s], &xml[e..]),
        None => xml.to_owned(),
    }
}

/// Byte range of the first element named exactly `tag`.
fn find_tag(xml: &str, tag: &str) -> Option<(usize, usize)> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(rel) = xml[from..].find(&open) {
        let start = from + rel;
        let next = xml[start + open.len()..].chars().next()?;
        if next == '>' || next == ' ' || next == '/' {
            let open_end = xml[start..].find('>')? + start;
            if xml[..open_end].ends_with('/') {
                return Some((start, open_end + 1));
            }
            let close = format!("</{tag}>");
            let close_start = xml[open_end..].find(&close)? + open_end;
            return Some((start, close_start + close.len()));
        }
        from = start + open.len();
    }
    None
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        // Multi-language fields: [{"id": "1", "value": "..."}]
        Value::Array(a) => a.first().map(|x| s(&x["value"])).unwrap_or_default(),
        _ => String::new(),
    }
}

fn f(v: &Value) -> Option<f64> {
    s(v).parse().ok()
}

fn non_empty(v: String) -> Option<String> {
    (!v.is_empty()).then_some(v)
}

#[async_trait]
impl Connector for PrestaShop {
    async fn fetch_catalog(&self) -> Result<Catalog> {
        let products = self
            .get_json(
                "products",
                "[id,name,reference,price,active,id_default_image]",
            )
            .await?;
        let combos = self
            .get_json("combinations", "[id,id_product,reference,price]")
            .await?;
        let stock = self
            .get_json(
                "stock_availables",
                "[id,id_product,id_product_attribute,quantity]",
            )
            .await?;

        let mut stock_by: HashMap<(String, String), (String, i64)> = HashMap::new();
        for st in &stock {
            let qty = s(&st["quantity"]).parse().unwrap_or(0);
            stock_by.insert(
                (s(&st["id_product"]), s(&st["id_product_attribute"])),
                (s(&st["id"]), qty),
            );
        }
        let mut combos_by: HashMap<String, Vec<&Value>> = HashMap::new();
        for c in &combos {
            combos_by.entry(s(&c["id_product"])).or_default().push(c);
        }

        let mut listings = Vec::new();
        for p in &products {
            let pid = s(&p["id"]);
            let name = s(&p["name"]);
            let base_price = f(&p["price"]).unwrap_or(0.0);
            let status = if s(&p["active"]) == "1" {
                "active"
            } else {
                "draft"
            };
            let image = non_empty(s(&p["id_default_image"]))
                .filter(|i| i != "0")
                .map(|img| format!("{}/api/images/products/{pid}/{img}", self.base));
            let stock_for = |attr: &str| -> Vec<RemoteStock> {
                stock_by
                    .get(&(pid.clone(), attr.to_owned()))
                    .map(|(id, q)| RemoteStock {
                        location_external_id: DEFAULT_LOCATION.into(),
                        stock_ref: Some(id.clone()),
                        quantity: *q,
                    })
                    .into_iter()
                    .collect()
            };
            match combos_by.get(&pid) {
                Some(cs) if !cs.is_empty() => {
                    for c in cs {
                        let cid = s(&c["id"]);
                        let reference =
                            non_empty(s(&c["reference"])).or_else(|| non_empty(s(&p["reference"])));
                        listings.push(RemoteListing {
                            external_product_id: pid.clone(),
                            external_variant_id: cid.clone(),
                            inventory_ref: None,
                            sku: reference,
                            title: format!("{name} / #{cid}"),
                            price: Some(base_price + f(&c["price"]).unwrap_or(0.0)),
                            status: Some(status.into()),
                            image_url: image.clone(),
                            stock: stock_for(&cid),
                        });
                    }
                }
                _ => listings.push(RemoteListing {
                    external_product_id: pid.clone(),
                    external_variant_id: String::new(),
                    inventory_ref: None,
                    sku: non_empty(s(&p["reference"])),
                    title: name,
                    price: Some(base_price),
                    status: Some(status.into()),
                    image_url: image,
                    stock: stock_for("0"),
                }),
            }
        }
        Ok(Catalog {
            locations: vec![RemoteLocation {
                external_id: DEFAULT_LOCATION.into(),
                name: "Shop stock".into(),
            }],
            listings,
        })
    }

    async fn fetch_orders(&self, since: &str) -> Result<Vec<RemoteOrderLine>> {
        let orders = self.get_json("orders", "[id,date_add]").await?;
        let since_cmp = since.replace('T', " ");
        let dates: HashMap<String, String> = orders
            .iter()
            .map(|o| (s(&o["id"]), s(&o["date_add"])))
            .filter(|(_, d)| d.as_str() >= since_cmp.as_str())
            .collect();
        let details = self
            .get_json(
                "order_details",
                "[id,id_order,product_id,product_attribute_id,product_reference,product_quantity,unit_price_tax_incl]",
            )
            .await?;
        Ok(details
            .iter()
            .filter_map(|d| {
                let order = s(&d["id_order"]);
                let date = dates.get(&order)?;
                let attr = s(&d["product_attribute_id"]);
                Some(RemoteOrderLine {
                    external_order_id: order,
                    external_line_id: s(&d["id"]),
                    external_product_id: s(&d["product_id"]),
                    external_variant_id: if attr == "0" { String::new() } else { attr },
                    sku: non_empty(s(&d["product_reference"])),
                    quantity: s(&d["product_quantity"]).parse().unwrap_or(0),
                    unit_price: f(&d["unit_price_tax_incl"]),
                    ordered_at: format!("{}Z", date.replace(' ', "T")),
                })
            })
            .collect())
    }

    async fn set_stock(
        &self,
        _listing: &ListingRef,
        _location: &str,
        stock_ref: Option<&str>,
        _current: i64,
        quantity: i64,
    ) -> Result<()> {
        let id = stock_ref.context("listing has no PrestaShop stock_available id")?;
        self.patch(
            "stock_availables",
            id,
            &[("quantity", quantity.to_string())],
        )
        .await
    }

    async fn set_price(&self, listing: &ListingRef, price: f64) -> Result<()> {
        if listing.external_variant_id.is_empty() {
            return self
                .patch(
                    "products",
                    &listing.external_product_id,
                    &[("price", format!("{price:.6}"))],
                )
                .await;
        }
        // Combination prices are an impact on the product's base price.
        let products = self.get_json("products", "[id,price]").await?;
        let base = products
            .iter()
            .find(|p| s(&p["id"]) == listing.external_product_id)
            .and_then(|p| f(&p["price"]))
            .unwrap_or(0.0);
        self.patch(
            "combinations",
            &listing.external_variant_id,
            &[("price", format!("{:.6}", price - base))],
        )
        .await
    }

    async fn set_status(&self, listing: &ListingRef, status: &str) -> Result<()> {
        let active = if status == "active" { "1" } else { "0" };
        self.patch(
            "products",
            &listing.external_product_id,
            &[("active", active.into())],
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_xml_fields() {
        let xml = "<prestashop><stock_available><id><![CDATA[3]]></id><quantity><![CDATA[5]]></quantity><quantity_x>1</quantity_x></stock_available></prestashop>";
        let out = replace_tag(xml, "quantity", "12").unwrap();
        assert!(out.contains("<quantity><![CDATA[12]]></quantity>"));
        assert!(out.contains("<quantity_x>1</quantity_x>"));
        let out = remove_tag(
            "<a><manufacturer_name notFilterable=\"true\">x</manufacturer_name><b/></a>",
            "manufacturer_name",
        );
        assert_eq!(out, "<a><b/></a>");
        assert_eq!(
            replace_tag("<a><price/></a>", "price", "1.5").unwrap(),
            "<a><price><![CDATA[1.5]]></price></a>"
        );
    }
}
