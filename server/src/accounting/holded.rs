//! Holded Invoicing API v1 (`/api/invoicing/v1`), API key in the `key` header.
//! Document lines carry the SKU directly, so items need no separate lookup.

use super::{Books, Config, Doc, Party};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct Holded {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl Holded {
    pub fn new(c: Config) -> Result<Self> {
        Ok(Self {
            http: super::http()?,
            base: format!("{}/api/invoicing/v1", c.base_url.trim_end_matches('/')),
            key: c.credential,
        })
    }

    async fn post(&self, path: &str, body: Value) -> Result<String> {
        let resp = self
            .http
            .post(format!("{}/{path}", self.base))
            .header("key", &self.key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() || v["status"].as_i64() == Some(0) {
            let msg = v["info"].as_str().unwrap_or("no details");
            return Err(anyhow!("Holded {path} failed ({status}): {msg}"));
        }
        v["id"]
            .as_str()
            .map(str::to_owned)
            .with_context(|| format!("Holded {path} returned no id"))
    }

    async fn document(&self, doc_type: &str, doc: &Doc) -> Result<String> {
        let date = chrono::NaiveDate::parse_from_str(&doc.date, "%Y-%m-%d")?
            .and_hms_opt(12, 0, 0)
            .context("bad date")?
            .and_utc()
            .timestamp();
        let items: Vec<Value> = doc
            .lines
            .iter()
            .map(|l| json!({ "name": l.name, "sku": l.sku, "units": l.quantity, "subtotal": l.unit_price }))
            .collect();
        self.post(
            &format!("documents/{doc_type}"),
            json!({
                "contactId": doc.contact, "date": date, "invoiceNum": doc.number,
                "notes": doc.note, "items": items,
            }),
        )
        .await
    }
}

#[async_trait]
impl Books for Holded {
    async fn contact(&self, party: Party, name: &str, email: Option<&str>) -> Result<String> {
        let contacts: Vec<Value> = self
            .http
            .get(format!("{}/contacts", self.base))
            .header("key", &self.key)
            .send()
            .await?
            .error_for_status()
            .context("Holded contacts lookup failed")?
            .json()
            .await?;
        if let Some(id) = contacts
            .iter()
            .find(|c| {
                c["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .and_then(|c| c["id"].as_str())
        {
            return Ok(id.to_owned());
        }
        let kind = match party {
            Party::Supplier => "supplier",
            Party::Customer => "client",
        };
        self.post(
            "contacts",
            json!({ "name": name, "email": email, "type": kind }),
        )
        .await
    }

    async fn item(&self, sku: &str, _name: &str) -> Result<String> {
        Ok(sku.to_owned())
    }

    async fn find_document(&self, party: Party, doc: &Doc) -> Result<Option<String>> {
        let doc_type = match party {
            Party::Supplier => "purchase",
            Party::Customer => "invoice",
        };
        let docs: Vec<Value> = self
            .http
            .get(format!("{}/documents/{doc_type}", self.base))
            .header("key", &self.key)
            .send()
            .await?
            .error_for_status()
            .context("Holded documents lookup failed")?
            .json()
            .await?;
        Ok(docs
            .iter()
            .find(|d| {
                d["docNumber"].as_str() == Some(doc.number.as_str())
                    && d["contact"].as_str() == Some(doc.contact.as_str())
            })
            .and_then(|d| d["id"].as_str())
            .map(str::to_owned))
    }

    async fn create_bill(&self, doc: &Doc) -> Result<String> {
        self.document("purchase", doc).await
    }

    async fn create_invoice(&self, doc: &Doc) -> Result<String> {
        self.document("invoice", doc).await
    }
}
