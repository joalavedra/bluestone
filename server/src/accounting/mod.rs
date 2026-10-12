//! Accounting sync behind one interface: purchase orders become supplier bills, sales orders
//! become customer invoices. QuickBooks and Holded implement [`Books`]; add a system by
//! implementing it and extending [`build`].

pub mod holded;
pub mod quickbooks;

use anyhow::{Result, bail};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Party {
    Supplier,
    Customer,
}

impl Party {
    pub fn as_str(self) -> &'static str {
        match self {
            Party::Supplier => "supplier",
            Party::Customer => "customer",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Line {
    /// Accounting-side item id from [`Books::item`].
    pub item: String,
    pub sku: String,
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
}

#[derive(Debug, Clone)]
pub struct Doc {
    /// Accounting-side contact id from [`Books::contact`].
    pub contact: String,
    /// Our reference, e.g. `PO-12`.
    pub number: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub note: Option<String>,
    pub lines: Vec<Line>,
}

#[async_trait]
pub trait Books: Send + Sync {
    /// Finds the supplier/customer by name, creating it if missing; returns its id.
    async fn contact(&self, party: Party, name: &str, email: Option<&str>) -> Result<String>;
    /// Finds the item by SKU, creating it if missing; returns its id.
    async fn item(&self, sku: &str, name: &str) -> Result<String>;
    async fn create_bill(&self, doc: &Doc) -> Result<String>;
    async fn create_invoice(&self, doc: &Doc) -> Result<String>;
    /// Finds a bill/invoice already created for this document number and contact, so a retry after
    /// a lost response doesn't create it twice.
    async fn find_document(&self, party: Party, doc: &Doc) -> Result<Option<String>>;
    /// Read-only call proving the credentials work; returns a short description (e.g. company name).
    async fn check(&self) -> Result<String>;
}

pub struct Config {
    pub kind: String,
    pub base_url: String,
    pub realm: Option<String>,
    pub income_account: Option<String>,
    pub expense_account: Option<String>,
    pub credential: String,
}

pub const KINDS: &[&str] = &["quickbooks", "holded"];

pub fn default_base_url(kind: &str) -> Option<&'static str> {
    match kind {
        "quickbooks" => Some("https://quickbooks.api.intuit.com"),
        "holded" => Some("https://api.holded.com"),
        _ => None,
    }
}

pub fn build(c: Config) -> Result<Box<dyn Books>> {
    Ok(match c.kind.as_str() {
        "quickbooks" => Box::new(quickbooks::QuickBooks::new(c)?),
        "holded" => Box::new(holded::Holded::new(c)?),
        k => bail!("unknown accounting system `{k}`"),
    })
}

pub(crate) fn amount(quantity: f64, unit_price: f64) -> f64 {
    (quantity * unit_price * 100.0).round() / 100.0
}

/// HTTP client with a request timeout, so an export can't hang past its claim.
pub fn http() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?)
}

pub fn label(kind: &str) -> &str {
    match kind {
        "quickbooks" => "QuickBooks",
        "holded" => "Holded",
        k => k,
    }
}
