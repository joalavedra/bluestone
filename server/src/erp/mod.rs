//! ERP sync behind one interface, like accounting: products (ERP code, SKU, stock) and customer
//! invoices are read, confirmed sales orders are pushed as ERP orders. Holded implements [`Erp`];
//! add an ERP by implementing it and extending [`build`].

pub mod holded;

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Product {
    /// ERP-side product id/code.
    pub code: String,
    pub sku: Option<String>,
    pub name: String,
    pub stock: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InvoiceLine {
    pub sku: Option<String>,
    pub name: String,
    pub quantity: f64,
    /// Line amount before tax.
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Invoice {
    pub id: String,
    pub number: Option<String>,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub customer: String,
    /// ISO 3166-1 alpha-2 country of the customer's billing address.
    pub country: Option<String>,
    pub currency: Option<String>,
    pub total: f64,
    pub lines: Vec<InvoiceLine>,
}

#[derive(Debug, Clone)]
pub struct OrderLine {
    pub sku: String,
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
}

#[derive(Debug, Clone)]
pub struct Order {
    /// Our reference, e.g. `SO-12`.
    pub number: String,
    /// `YYYY-MM-DD`.
    pub date: String,
    pub customer: String,
    pub email: Option<String>,
    pub note: Option<String>,
    pub lines: Vec<OrderLine>,
}

#[async_trait]
pub trait Erp: Send + Sync {
    /// Read-only call proving the credentials work; returns a short description.
    async fn check(&self) -> Result<String>;
    async fn products(&self) -> Result<Vec<Product>>;
    /// Customer invoices dated `from..=to` (`YYYY-MM-DD`), with the customer's country.
    async fn invoices(&self, from: &str, to: &str) -> Result<Vec<Invoice>>;
    /// Finds an order already created for this number, so a retry after a lost response doesn't
    /// create it twice.
    async fn find_order(&self, number: &str) -> Result<Option<String>>;
    async fn create_order(&self, order: &Order) -> Result<String>;
}

pub const KINDS: &[&str] = &["holded"];

pub fn default_base_url(kind: &str) -> Option<&'static str> {
    match kind {
        "holded" => Some("https://api.holded.com"),
        _ => None,
    }
}

pub fn label(kind: &str) -> &str {
    match kind {
        "holded" => "Holded",
        k => k,
    }
}

pub fn build(kind: &str, base_url: &str, credential: String) -> Result<Box<dyn Erp>> {
    Ok(match kind {
        "holded" => Box::new(holded::HoldedErp::new(base_url, credential)?),
        k => bail!("unknown ERP `{k}`"),
    })
}
