pub mod prestashop;
pub mod shopify;

use anyhow::Result;
use async_trait::async_trait;

#[derive(Debug, Clone)]
pub struct RemoteLocation {
    pub external_id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct RemoteStock {
    pub location_external_id: String,
    /// Connector-specific handle used to write this stock row back (e.g. PrestaShop stock_available id).
    pub stock_ref: Option<String>,
    pub quantity: i64,
}

#[derive(Debug, Clone)]
pub struct RemoteListing {
    pub external_product_id: String,
    pub external_variant_id: String,
    /// Connector-specific inventory handle (Shopify InventoryItem gid).
    pub inventory_ref: Option<String>,
    pub sku: Option<String>,
    pub title: String,
    pub price: Option<f64>,
    pub status: Option<String>,
    pub image_url: Option<String>,
    pub stock: Vec<RemoteStock>,
}

#[derive(Debug, Clone)]
pub struct RemoteOrderLine {
    pub external_order_id: String,
    pub external_line_id: String,
    pub external_product_id: String,
    pub external_variant_id: String,
    pub sku: Option<String>,
    pub quantity: i64,
    pub unit_price: Option<f64>,
    pub ordered_at: String,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub locations: Vec<RemoteLocation>,
    pub listings: Vec<RemoteListing>,
}

/// Target of a write: everything a connector needs to address one listing.
#[derive(Debug, Clone)]
pub struct ListingRef {
    pub external_product_id: String,
    pub external_variant_id: String,
    pub inventory_ref: Option<String>,
}

#[async_trait]
pub trait Connector: Send + Sync {
    async fn fetch_catalog(&self) -> Result<Catalog>;
    async fn fetch_orders(&self, since: &str) -> Result<Vec<RemoteOrderLine>>;
    async fn set_stock(
        &self,
        listing: &ListingRef,
        location_external_id: &str,
        stock_ref: Option<&str>,
        current: i64,
        quantity: i64,
    ) -> Result<()>;
    async fn set_price(&self, listing: &ListingRef, price: f64) -> Result<()>;
    async fn set_status(&self, listing: &ListingRef, status: &str) -> Result<()>;
}

pub fn build(kind: &str, base_url: &str, credential: String) -> Result<Box<dyn Connector>> {
    match kind {
        "shopify" => Ok(Box::new(shopify::Shopify::new(base_url, credential))),
        "prestashop" => Ok(Box::new(prestashop::PrestaShop::new(base_url, credential))),
        other => anyhow::bail!("unknown channel kind `{other}`"),
    }
}
