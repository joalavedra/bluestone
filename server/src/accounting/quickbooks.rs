//! QuickBooks Online Accounting API v3 (`/v3/company/{realm}/...`), OAuth bearer token.

use super::{Books, Config, Doc, Party, amount};
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use serde_json::{Value, json};

const MINOR_VERSION: &str = "75";

pub struct QuickBooks {
    http: reqwest::Client,
    base: String,
    token: String,
    income_account: String,
    expense_account: String,
}

impl QuickBooks {
    pub fn new(c: Config) -> Result<Self> {
        let realm = c
            .realm
            .context("QuickBooks needs the company id (--realm)")?;
        Ok(Self {
            http: reqwest::Client::new(),
            base: format!("{}/v3/company/{realm}", c.base_url.trim_end_matches('/')),
            token: c.credential,
            income_account: c
                .income_account
                .context("QuickBooks needs --income-account to create items")?,
            expense_account: c
                .expense_account
                .context("QuickBooks needs --expense-account to create items")?,
        })
    }

    async fn find(&self, entity: &str, field: &str, value: &str) -> Result<Option<String>> {
        let q = format!(
            "select Id from {entity} where {field} = '{}'",
            value.replace('\\', "\\\\").replace('\'', "\\'")
        );
        let v: Value = self
            .http
            .get(format!("{}/query", self.base))
            .query(&[("query", q.as_str()), ("minorversion", MINOR_VERSION)])
            .bearer_auth(&self.token)
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("QuickBooks {entity} query failed"))?
            .json()
            .await?;
        Ok(v["QueryResponse"][entity][0]["Id"]
            .as_str()
            .map(str::to_owned))
    }

    async fn create(&self, entity: &str, body: Value) -> Result<String> {
        let resp = self
            .http
            .post(format!("{}/{}", self.base, entity.to_lowercase()))
            .query(&[("minorversion", MINOR_VERSION)])
            .bearer_auth(&self.token)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let v: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = v["Fault"]["Error"][0]["Detail"]
                .as_str()
                .or(v["Fault"]["Error"][0]["Message"].as_str())
                .unwrap_or("no details");
            return Err(anyhow!(
                "QuickBooks {entity} create failed ({status}): {msg}"
            ));
        }
        v[entity]["Id"]
            .as_str()
            .map(str::to_owned)
            .with_context(|| format!("QuickBooks returned no {entity} id"))
    }
}

fn lines(doc: &Doc, detail: &str) -> Vec<Value> {
    doc.lines
        .iter()
        .map(|l| {
            json!({
                "DetailType": detail,
                "Amount": amount(l.quantity, l.unit_price),
                "Description": format!("{} · {}", l.sku, l.name),
                detail: { "ItemRef": { "value": l.item }, "Qty": l.quantity, "UnitPrice": l.unit_price },
            })
        })
        .collect()
}

#[async_trait]
impl Books for QuickBooks {
    async fn contact(&self, party: Party, name: &str, email: Option<&str>) -> Result<String> {
        let entity = match party {
            Party::Supplier => "Vendor",
            Party::Customer => "Customer",
        };
        if let Some(id) = self.find(entity, "DisplayName", name).await? {
            return Ok(id);
        }
        let mut body = json!({ "DisplayName": name });
        if let Some(e) = email {
            body["PrimaryEmailAddr"] = json!({ "Address": e });
        }
        self.create(entity, body).await
    }

    async fn item(&self, sku: &str, name: &str) -> Result<String> {
        if let Some(id) = self.find("Item", "Sku", sku).await? {
            return Ok(id);
        }
        self.create(
            "Item",
            json!({
                "Name": sku, "Sku": sku, "Description": name, "Type": "NonInventory",
                "IncomeAccountRef": { "value": self.income_account },
                "ExpenseAccountRef": { "value": self.expense_account },
            }),
        )
        .await
    }

    async fn create_bill(&self, doc: &Doc) -> Result<String> {
        self.create(
            "Bill",
            json!({
                "VendorRef": { "value": doc.contact }, "DocNumber": doc.number, "TxnDate": doc.date,
                "PrivateNote": doc.note, "Line": lines(doc, "ItemBasedExpenseLineDetail"),
            }),
        )
        .await
    }

    async fn create_invoice(&self, doc: &Doc) -> Result<String> {
        self.create(
            "Invoice",
            json!({
                "CustomerRef": { "value": doc.contact }, "DocNumber": doc.number, "TxnDate": doc.date,
                "PrivateNote": doc.note, "Line": lines(doc, "SalesItemLineDetail"),
            }),
        )
        .await
    }
}
