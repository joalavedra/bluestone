//! Holded as an ERP: products, invoices and sales orders on the same invoicing API v1 and key as
//! the accounting adapter.

use super::{Erp, Invoice, InvoiceLine, Order, Product};
use crate::accounting::{self, Books, Doc, Line, Party, holded::Holded};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

pub struct HoldedErp(Holded);

impl HoldedErp {
    pub fn new(base_url: &str, key: String) -> Result<Self> {
        Ok(Self(Holded::new(accounting::Config {
            kind: "holded".into(),
            base_url: base_url.into(),
            realm: None,
            income_account: None,
            expense_account: None,
            credential: key,
        })?))
    }
}

fn timestamp(day: &str, end_of_day: bool) -> Result<String> {
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .with_context(|| format!("bad date `{day}`, expected YYYY-MM-DD"))?;
    let t = if end_of_day {
        d.and_hms_opt(23, 59, 59)
    } else {
        d.and_hms_opt(0, 0, 0)
    };
    Ok(t.context("bad date")?.and_utc().timestamp().to_string())
}

fn text(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

/// A top-level list; anything else means Holded didn't return what we asked for.
fn array<'a>(v: &'a Value, what: &str) -> Result<&'a [Value]> {
    v.as_array()
        .map(Vec::as_slice)
        .with_context(|| format!("Holded {what}: expected a list"))
}

#[async_trait]
impl Erp for HoldedErp {
    async fn check(&self) -> Result<String> {
        let products = self.0.get("products", &[]).await?;
        Ok(format!(
            "API key works · {} products",
            array(&products, "products")?.len()
        ))
    }

    async fn products(&self) -> Result<Vec<Product>> {
        let v = self.0.get("products", &[]).await?;
        Ok(array(&v, "products")?
            .iter()
            .filter_map(|p| {
                Some(Product {
                    code: text(&p["id"])?,
                    sku: text(&p["sku"]),
                    name: text(&p["name"]).unwrap_or_default(),
                    stock: p["stock"].as_f64(),
                })
            })
            .collect())
    }

    async fn invoices(&self, from: &str, to: &str) -> Result<Vec<Invoice>> {
        let query = [
            ("starttmp", timestamp(from, false)?),
            ("endtmp", timestamp(to, true)?),
        ];
        let docs = self.0.get("documents/invoice", &query).await?;
        let mut countries: HashMap<String, Option<String>> = HashMap::new();
        let mut out = Vec::new();
        for d in array(&docs, "invoices")? {
            let contact = text(&d["contact"]).unwrap_or_default();
            if !contact.is_empty() && !countries.contains_key(&contact) {
                let c = self.0.get(&format!("contacts/{contact}"), &[]).await?;
                countries.insert(contact.clone(), text(&c["billAddress"]["countryCode"]));
            }
            let lines = list(&d["products"])
                .iter()
                .map(|l| {
                    let quantity = l["units"].as_f64().unwrap_or(0.0);
                    InvoiceLine {
                        sku: text(&l["sku"]),
                        name: text(&l["name"]).unwrap_or_default(),
                        quantity,
                        amount: accounting::amount(quantity, l["price"].as_f64().unwrap_or(0.0)),
                    }
                })
                .collect();
            out.push(Invoice {
                id: text(&d["id"]).unwrap_or_default(),
                number: text(&d["docNumber"]),
                date: d["date"]
                    .as_i64()
                    .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
                    .map(|t| t.format("%Y-%m-%d").to_string())
                    .unwrap_or_default(),
                customer: text(&d["contactName"]).unwrap_or_else(|| contact.clone()),
                country: countries.get(&contact).cloned().flatten(),
                currency: text(&d["currency"]).map(|c| c.to_uppercase()),
                total: d["total"].as_f64().unwrap_or(0.0),
                lines,
            });
        }
        Ok(out)
    }

    async fn find_order(&self, number: &str) -> Result<Option<String>> {
        let docs = self.0.get("documents/salesorder", &[]).await?;
        Ok(array(&docs, "sales orders")?
            .iter()
            .find(|d| d["docNumber"].as_str() == Some(number))
            .and_then(|d| text(&d["id"])))
    }

    async fn create_order(&self, o: &Order) -> Result<String> {
        let contact = self
            .0
            .contact(Party::Customer, &o.customer, o.email.as_deref())
            .await?;
        let doc = Doc {
            contact,
            number: o.number.clone(),
            date: o.date.clone(),
            note: o.note.clone(),
            lines: o
                .lines
                .iter()
                .map(|l| Line {
                    item: l.sku.clone(),
                    sku: l.sku.clone(),
                    name: l.name.clone(),
                    quantity: l.quantity,
                    unit_price: l.unit_price,
                })
                .collect(),
        };
        self.0.document("salesorder", &doc).await
    }
}
