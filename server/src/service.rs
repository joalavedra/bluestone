//! Domain logic shared by the REST API, the MCP server and the CLI.
use crate::auth::{Principal, Scope};
use crate::connectors::{self, ListingRef};
use anyhow::Context;
use chrono::{Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};
use std::collections::HashMap;

pub const DEFAULT_LEAD_TIME_DAYS: i64 = 14;
const VELOCITY_WINDOW_DAYS: i64 = 30;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    Conflict(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ServiceError {
    fn from(e: sqlx::Error) -> Self {
        Self::Internal(e.into())
    }
}

pub type SResult<T> = Result<T, ServiceError>;

pub(crate) fn invalid<T>(msg: impl Into<String>) -> SResult<T> {
    Err(ServiceError::Invalid(msg.into()))
}

pub(crate) fn now() -> String {
    Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub(crate) fn days_ago(d: i64) -> String {
    (Utc::now() - Duration::days(d))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}

#[derive(Clone)]
pub struct Service {
    pub pool: SqlitePool,
}

// ---------- views ----------

#[derive(Debug, Serialize, Clone)]
pub struct ChannelInfo {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub base_url: String,
    pub last_synced_at: Option<String>,
    pub last_sync_error: Option<String>,
    pub listing_count: i64,
}

#[derive(Debug, Serialize)]
pub struct BrandSummary {
    pub id: i64,
    pub name: String,
    pub item_count: i64,
    pub low_stock_count: i64,
    pub out_of_stock_count: i64,
    pub stock_value: f64,
    /// `mirror`: the stores own stock. `master`: Bluestone's ledger owns stock and pushes it out.
    pub stock_mode: String,
    pub channels: Vec<ChannelInfo>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ItemSummary {
    pub id: i64,
    pub brand: String,
    pub sku: String,
    pub name: String,
    pub supplier: Option<String>,
    pub tags: Vec<String>,
    pub channels: Vec<String>,
    pub on_hand: i64,
    pub sold_30d: i64,
    pub daily_velocity: f64,
    pub days_cover: Option<f64>,
    pub reorder_point: Option<i64>,
    pub target_stock: Option<i64>,
    pub lead_time_days: i64,
    pub suggested_reorder_qty: i64,
    pub unit_cost: Option<f64>,
    pub price: Option<f64>,
    pub image_url: Option<String>,
    /// `out`, `low` or `ok`.
    pub status: String,
    /// `ses` (regular seller), `tsb` (intermittent) or `none`; drives `daily_velocity`.
    pub forecast_method: String,
    pub safety_stock: i64,
    /// Lead-time demand + safety stock; used when no manual `reorder_point` is set.
    pub forecast_reorder_point: i64,
}

#[derive(FromRow)]
struct ItemRow {
    id: i64,
    brand: String,
    sku: String,
    name: String,
    supplier: Option<String>,
    supplier_lead: Option<i64>,
    reorder_point: Option<i64>,
    target_stock: Option<i64>,
    lead_time_days: Option<i64>,
    unit_cost: Option<f64>,
    on_hand: i64,
    sold_30d: i64,
    tags: Option<String>,
    channels: Option<String>,
    image_url: Option<String>,
    price: Option<f64>,
    sales_90d: Option<String>,
}

/// Daily units for the last 90 days (oldest first) from `day:qty` pairs.
fn daily_series(raw: Option<&str>) -> Vec<f64> {
    let today = chrono::Utc::now().date_naive();
    let mut y = vec![0.0; 90];
    for (day, qty) in raw
        .unwrap_or_default()
        .split(',')
        .filter_map(|p| p.split_once(':'))
    {
        if let (Ok(d), Ok(q)) = (
            chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d"),
            qty.parse::<f64>(),
        ) {
            let ago = (today - d).num_days();
            if (0..90).contains(&ago) {
                y[89 - ago as usize] += q;
            }
        }
    }
    y
}

impl From<ItemRow> for ItemSummary {
    fn from(r: ItemRow) -> Self {
        let lead = r
            .lead_time_days
            .or(r.supplier_lead)
            .unwrap_or(DEFAULT_LEAD_TIME_DAYS);
        let fc = crate::forecast::forecast(&daily_series(r.sales_90d.as_deref()));
        let velocity = fc.rate;
        let safety = crate::forecast::safety_stock(&fc, lead);
        let forecast_rop = crate::forecast::reorder_point(&fc, lead);
        let days_cover =
            (velocity > 0.0).then(|| (r.on_hand as f64 / velocity * 10.0).round() / 10.0);
        let low = match r.reorder_point {
            Some(rp) => r.on_hand <= rp,
            None => velocity > 0.0 && r.on_hand <= forecast_rop,
        };
        let status = if r.on_hand <= 0 {
            "out"
        } else if low {
            "low"
        } else {
            "ok"
        };
        let suggested = match r.target_stock {
            Some(t) => (t - r.on_hand).max(0),
            None if status != "ok" => {
                ((velocity * (lead + VELOCITY_WINDOW_DAYS) as f64).ceil() as i64 + safety
                    - r.on_hand)
                    .max(r.reorder_point.unwrap_or(0) - r.on_hand)
                    .max(0)
            }
            None => 0,
        };
        let split = |s: Option<String>| -> Vec<String> {
            let mut v: Vec<String> = s
                .unwrap_or_default()
                .split(',')
                .filter(|x| !x.is_empty())
                .map(Into::into)
                .collect();
            v.sort();
            v.dedup();
            v
        };
        Self {
            id: r.id,
            brand: r.brand,
            sku: r.sku,
            name: r.name,
            supplier: r.supplier,
            tags: split(r.tags),
            channels: split(r.channels),
            on_hand: r.on_hand,
            sold_30d: r.sold_30d,
            daily_velocity: (velocity * 100.0).round() / 100.0,
            days_cover,
            reorder_point: r.reorder_point,
            target_stock: r.target_stock,
            lead_time_days: lead,
            suggested_reorder_qty: suggested,
            unit_cost: r.unit_cost,
            price: r.price,
            image_url: r.image_url,
            status: status.into(),
            forecast_method: fc.method.into(),
            safety_stock: safety,
            forecast_reorder_point: forecast_rop,
        }
    }
}

const ITEM_SELECT: &str = r#"
SELECT i.id, b.name AS brand, i.sku, i.name, s.name AS supplier, s.lead_time_days AS supplier_lead,
  i.reorder_point, i.target_stock, i.lead_time_days, i.unit_cost,
  CASE WHEN b.stock_mode = 'master'
    THEN COALESCE((SELECT SUM(s.delta) FROM stock_ledger s WHERE s.item_id = i.id), 0)
    ELSE COALESCE((SELECT SUM(ls.quantity) FROM listing_stock ls JOIN listings l ON l.id = ls.listing_id WHERE l.item_id = i.id), 0)
  END AS on_hand,
  COALESCE((SELECT SUM(o.quantity) FROM order_lines o JOIN listings l ON l.id = o.listing_id WHERE l.item_id = i.id AND o.ordered_at >= ?1), 0) AS sold_30d,
  (SELECT GROUP_CONCAT(t.name) FROM item_tags it JOIN tags t ON t.id = it.tag_id WHERE it.item_id = i.id) AS tags,
  (SELECT GROUP_CONCAT(c.kind || ':' || c.name) FROM listings l JOIN channels c ON c.id = l.channel_id WHERE l.item_id = i.id) AS channels,
  (SELECT l.image_url FROM listings l WHERE l.item_id = i.id AND l.image_url IS NOT NULL LIMIT 1) AS image_url,
  (SELECT MIN(l.price) FROM listings l WHERE l.item_id = i.id) AS price,
  (SELECT GROUP_CONCAT(substr(o.ordered_at, 1, 10) || ':' || o.quantity) FROM order_lines o JOIN listings l ON l.id = o.listing_id
     WHERE l.item_id = i.id AND o.ordered_at >= strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '-90 days')) AS sales_90d
FROM items i JOIN brands b ON b.id = i.brand_id LEFT JOIN suppliers s ON s.id = i.supplier_id
WHERE i.archived = 0"#;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ItemFilter {
    /// Free text matched against SKU and name.
    pub query: Option<String>,
    /// Brand name.
    pub brand: Option<String>,
    pub tag: Option<String>,
    pub supplier: Option<String>,
    /// `out`, `low` or `ok`. `attention` returns both `out` and `low`.
    pub status: Option<String>,
    /// Only items with fewer than this many days of stock left.
    pub max_days_cover: Option<f64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct StockAt {
    pub location: String,
    pub quantity: i64,
}

#[derive(Debug, Serialize)]
pub struct ListingInfo {
    pub id: i64,
    pub channel_id: i64,
    pub channel: String,
    pub kind: String,
    pub external_product_id: String,
    pub external_variant_id: String,
    pub sku: Option<String>,
    pub title: String,
    pub price: Option<f64>,
    pub status: Option<String>,
    pub stock: Vec<StockAt>,
    pub synced_at: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Note {
    pub id: i64,
    pub body: String,
    pub actor: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct DayPoint {
    pub day: String,
    pub units: i64,
}

#[derive(Debug, Serialize)]
pub struct ItemDetail {
    #[serde(flatten)]
    pub summary: ItemSummary,
    pub listings: Vec<ListingInfo>,
    pub notes: Vec<Note>,
    pub sales_by_day: Vec<DayPoint>,
    pub proposals: Vec<ProposalView>,
    pub stock_mode: String,
    /// Master mode only: ledger quantity per warehouse and the latest ledger entries.
    pub warehouses: Vec<crate::ledger::StockLevel>,
    pub ledger: Vec<crate::ledger::LedgerEntry>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProposalView {
    pub id: i64,
    pub kind: String,
    pub status: String,
    pub item_id: Option<i64>,
    pub item_sku: Option<String>,
    pub item_name: Option<String>,
    pub brand: Option<String>,
    /// None for ledger proposals, which target a warehouse.
    pub listing_id: Option<i64>,
    pub channel: String,
    pub before: Value,
    pub after: Value,
    pub warnings: Vec<String>,
    pub rationale: Option<String>,
    pub actor: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(FromRow)]
struct ProposalRow {
    id: i64,
    kind: String,
    status: String,
    item_id: Option<i64>,
    item_sku: Option<String>,
    item_name: Option<String>,
    brand: Option<String>,
    listing_id: Option<i64>,
    channel: String,
    before_json: String,
    after_json: String,
    rationale: Option<String>,
    actor: String,
    decided_by: Option<String>,
    decided_at: Option<String>,
    error: Option<String>,
    created_at: String,
}

impl From<ProposalRow> for ProposalView {
    fn from(r: ProposalRow) -> Self {
        let before: Value = serde_json::from_str(&r.before_json).unwrap_or(Value::Null);
        let after: Value = serde_json::from_str(&r.after_json).unwrap_or(Value::Null);
        let warnings = policy_warnings(&r.kind, &before, &after);
        Self {
            id: r.id,
            kind: r.kind,
            status: r.status,
            item_id: r.item_id,
            item_sku: r.item_sku,
            item_name: r.item_name,
            brand: r.brand,
            listing_id: r.listing_id,
            channel: r.channel,
            before,
            after,
            warnings,
            rationale: r.rationale,
            actor: r.actor,
            decided_by: r.decided_by,
            decided_at: r.decided_at,
            error: r.error,
            created_at: r.created_at,
        }
    }
}

/// Soft policy: never blocks, but flags proposals a human should look at twice.
#[allow(clippy::collapsible_if, clippy::collapsible_match)]
fn policy_warnings(kind: &str, before: &Value, after: &Value) -> Vec<String> {
    let mut w = Vec::new();
    match kind {
        "price_change" => {
            if let (Some(b), Some(a)) = (before["price"].as_f64(), after["price"].as_f64()) {
                if b > 0.0 {
                    let pct = (a - b) / b * 100.0;
                    if pct.abs() >= 30.0 {
                        w.push(format!("price moves {pct:+.0}%"));
                    }
                }
            }
        }
        "stock_adjustment" | "ledger_adjustment" => {
            if let (Some(b), Some(a)) = (before["quantity"].as_i64(), after["quantity"].as_i64()) {
                if a < b && b - a >= 50 {
                    w.push(format!("removes {} units", b - a));
                }
                if a > b && a - b >= 500 {
                    w.push(format!("adds {} units", a - b));
                }
            }
        }
        "listing_status" => {
            if after["status"] == "archived" {
                w.push("archives the listing".into());
            }
        }
        _ => {}
    }
    w
}

const PROPOSAL_SELECT: &str = r#"
SELECT p.id, p.kind, p.status, p.item_id, i.sku AS item_sku, i.name AS item_name, b.name AS brand,
  p.listing_id, COALESCE(c.kind || ':' || c.name, 'bluestone:ledger') AS channel, p.before_json, p.after_json, p.rationale, p.actor,
  p.decided_by, p.decided_at, p.error, p.created_at
FROM proposals p
LEFT JOIN listings l ON l.id = p.listing_id
LEFT JOIN channels c ON c.id = l.channel_id
LEFT JOIN items i ON i.id = p.item_id
LEFT JOIN brands b ON b.id = i.brand_id"#;

#[derive(Debug, Serialize, FromRow)]
pub struct ActivityEntry {
    pub id: i64,
    pub actor_kind: String,
    pub actor: String,
    pub action: String,
    pub subject_type: Option<String>,
    pub subject_id: Option<i64>,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct TagCount {
    pub name: String,
    pub items: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Supplier {
    pub id: i64,
    pub name: String,
    pub email: Option<String>,
    pub lead_time_days: Option<i64>,
    pub notes: Option<String>,
    pub items: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct TopItem {
    pub sku: String,
    pub name: String,
    pub brand: String,
    pub units: i64,
    pub revenue: f64,
}

#[derive(Debug, Serialize)]
pub struct SalesSummary {
    pub period_days: i64,
    pub brand: Option<String>,
    pub units: i64,
    pub revenue: f64,
    pub orders: i64,
    pub by_day: Vec<DayPoint>,
    pub top_items: Vec<TopItem>,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    pub brands: Vec<BrandSummary>,
    pub items: i64,
    pub low_stock: i64,
    pub out_of_stock: i64,
    pub pending_proposals: i64,
    pub units_30d: i64,
    pub revenue_30d: f64,
    pub stock_value: f64,
    pub attention: Vec<ItemSummary>,
    pub recent_activity: Vec<ActivityEntry>,
}

#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub channel_id: i64,
    pub channel: String,
    pub listings: usize,
    pub new_items: usize,
    pub order_lines: usize,
    /// Present for brands in master mode: what the ledger did after the sync.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master: Option<crate::ledger::Reconcile>,
}

// ---------- inputs ----------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewChannel {
    pub brand: String,
    /// `shopify` or `prestashop`.
    pub kind: String,
    pub name: String,
    /// Shopify: https://<shop>.myshopify.com. PrestaShop: shop root URL.
    pub base_url: String,
    /// Name of the env var holding the access token / WebService key.
    pub credential_env: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeStock {
    /// Item id or SKU.
    pub item: String,
    pub brand: Option<String>,
    /// Channel name, required when the item is listed on several channels.
    pub channel: Option<String>,
    /// Location name, required when the listing has stock at several locations.
    pub location: Option<String>,
    /// Absolute new quantity. Use either this or `delta`.
    pub quantity: Option<i64>,
    /// Relative change, e.g. 24 or -3.
    pub delta: Option<i64>,
    pub rationale: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposePrice {
    pub item: String,
    pub brand: Option<String>,
    pub channel: Option<String>,
    pub price: f64,
    pub rationale: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeStatus {
    pub item: String,
    pub brand: Option<String>,
    pub channel: Option<String>,
    /// `active`, `draft` or `archived` (PrestaShop treats draft and archived as disabled).
    pub status: String,
    pub rationale: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ItemSettings {
    pub reorder_point: Option<i64>,
    pub target_stock: Option<i64>,
    pub lead_time_days: Option<i64>,
    pub unit_cost: Option<f64>,
}

impl Service {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn log(
        &self,
        who: &Principal,
        action: &str,
        subject: Option<(&str, i64)>,
        summary: impl Into<String>,
    ) -> SResult<()> {
        sqlx::query(
            "INSERT INTO activity (actor_kind, actor, action, subject_type, subject_id, summary) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&who.kind)
        .bind(&who.name)
        .bind(action)
        .bind(subject.map(|s| s.0))
        .bind(subject.map(|s| s.1))
        .bind(summary.into())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ---------- reads ----------

    async fn all_items(&self) -> SResult<Vec<ItemSummary>> {
        let rows: Vec<ItemRow> = sqlx::query_as(&format!("{ITEM_SELECT} ORDER BY b.name, i.name"))
            .bind(days_ago(VELOCITY_WINDOW_DAYS))
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn search_items(&self, f: &ItemFilter) -> SResult<Vec<ItemSummary>> {
        let q = f.query.as_deref().map(str::to_lowercase);
        let eq =
            |a: &str, b: &Option<String>| b.as_deref().is_none_or(|b| a.eq_ignore_ascii_case(b));
        let mut items: Vec<ItemSummary> = self
            .all_items()
            .await?
            .into_iter()
            .filter(|i| {
                q.as_deref().is_none_or(|q| {
                    i.sku.to_lowercase().contains(q) || i.name.to_lowercase().contains(q)
                }) && eq(&i.brand, &f.brand)
                    && f.tag
                        .as_deref()
                        .is_none_or(|t| i.tags.iter().any(|x| x.eq_ignore_ascii_case(t)))
                    && f.supplier.as_deref().is_none_or(|s| {
                        i.supplier
                            .as_deref()
                            .is_some_and(|x| x.eq_ignore_ascii_case(s))
                    })
                    && match f.status.as_deref() {
                        None | Some("") | Some("all") => true,
                        Some("attention") => i.status != "ok",
                        Some(s) => i.status == s,
                    }
                    && f.max_days_cover
                        .is_none_or(|m| i.on_hand <= 0 || i.days_cover.is_some_and(|d| d < m))
            })
            .collect();
        if f.status.as_deref() == Some("attention") || f.max_days_cover.is_some() {
            items.sort_by(|a, b| {
                a.days_cover
                    .unwrap_or(f64::MAX)
                    .total_cmp(&b.days_cover.unwrap_or(f64::MAX))
                    .then(a.on_hand.cmp(&b.on_hand))
            });
        }
        items.truncate(f.limit.unwrap_or(200).clamp(1, 1000) as usize);
        Ok(items)
    }

    pub async fn resolve_item(&self, item: &str, brand: Option<&str>) -> SResult<i64> {
        let item = item.trim();
        if let Ok(id) = item.parse::<i64>() {
            let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM items WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
            if let Some((id,)) = exists {
                return Ok(id);
            }
        }
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT i.id, b.name FROM items i JOIN brands b ON b.id = i.brand_id WHERE i.sku = ? COLLATE NOCASE AND (?2 IS NULL OR b.name = ?2 COLLATE NOCASE)",
        )
        .bind(item)
        .bind(brand)
        .fetch_all(&self.pool)
        .await?;
        match rows.as_slice() {
            [] => Err(ServiceError::NotFound(format!(
                "no item with id or SKU `{item}`"
            ))),
            [(id, _)] => Ok(*id),
            many => invalid(format!(
                "SKU `{item}` exists in several brands ({}); pass `brand`",
                many.iter()
                    .map(|r| r.1.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    pub async fn get_item(&self, id: i64) -> SResult<ItemDetail> {
        let row: ItemRow = sqlx::query_as(&format!("{ITEM_SELECT} AND i.id = ?2"))
            .bind(days_ago(VELOCITY_WINDOW_DAYS))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| ServiceError::NotFound(format!("item {id} not found")))?;
        let listings = self.listings_for(id).await?;
        let notes = sqlx::query_as(
            "SELECT id, body, actor, created_at FROM notes WHERE item_id = ? ORDER BY id DESC",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        let sales_by_day = sqlx::query_as(
            "SELECT substr(o.ordered_at, 1, 10) AS day, SUM(o.quantity) AS units FROM order_lines o JOIN listings l ON l.id = o.listing_id
             WHERE l.item_id = ? AND o.ordered_at >= ? GROUP BY day ORDER BY day",
        )
        .bind(id)
        .bind(days_ago(90))
        .fetch_all(&self.pool)
        .await?;
        let proposals = self.proposals_where("p.item_id = ?", Some(id), 20).await?;
        let stock_mode = self.item_stock_mode(id).await?;
        let (warehouses, ledger) = if stock_mode == "master" {
            (self.stock_levels(id).await?, self.ledger(id, 50).await?)
        } else {
            (Vec::new(), Vec::new())
        };
        Ok(ItemDetail {
            stock_mode,
            warehouses,
            ledger,
            summary: row.into(),
            listings,
            notes,
            sales_by_day,
            proposals,
        })
    }

    async fn listings_for(&self, item_id: i64) -> SResult<Vec<ListingInfo>> {
        #[derive(FromRow)]
        struct Row {
            id: i64,
            channel_id: i64,
            channel: String,
            kind: String,
            external_product_id: String,
            external_variant_id: String,
            sku: Option<String>,
            title: String,
            price: Option<f64>,
            status: Option<String>,
            synced_at: String,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT l.id, l.channel_id, c.name AS channel, c.kind, l.external_product_id, l.external_variant_id, l.sku, l.title, l.price, l.status, l.synced_at
             FROM listings l JOIN channels c ON c.id = l.channel_id WHERE l.item_id = ? ORDER BY c.name",
        )
        .bind(item_id)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for r in rows {
            let stock: Vec<(String, i64)> = sqlx::query_as(
                "SELECT lo.name, ls.quantity FROM listing_stock ls JOIN locations lo ON lo.id = ls.location_id WHERE ls.listing_id = ? ORDER BY lo.name",
            )
            .bind(r.id)
            .fetch_all(&self.pool)
            .await?;
            out.push(ListingInfo {
                id: r.id,
                channel_id: r.channel_id,
                channel: r.channel,
                kind: r.kind,
                external_product_id: r.external_product_id,
                external_variant_id: r.external_variant_id,
                sku: r.sku,
                title: r.title,
                price: r.price,
                status: r.status,
                stock: stock
                    .into_iter()
                    .map(|(location, quantity)| StockAt { location, quantity })
                    .collect(),
                synced_at: r.synced_at,
            });
        }
        Ok(out)
    }

    pub async fn channels(&self) -> SResult<Vec<(i64, ChannelInfo)>> {
        #[derive(FromRow)]
        struct Row {
            id: i64,
            brand_id: i64,
            kind: String,
            name: String,
            base_url: String,
            last_synced_at: Option<String>,
            last_sync_error: Option<String>,
            listing_count: i64,
        }
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT c.id, c.brand_id, c.kind, c.name, c.base_url, c.last_synced_at, c.last_sync_error,
               (SELECT COUNT(*) FROM listings l WHERE l.channel_id = c.id) AS listing_count
             FROM channels c ORDER BY c.name",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.brand_id,
                    ChannelInfo {
                        id: r.id,
                        kind: r.kind,
                        name: r.name,
                        base_url: r.base_url,
                        last_synced_at: r.last_synced_at,
                        last_sync_error: r.last_sync_error,
                        listing_count: r.listing_count,
                    },
                )
            })
            .collect())
    }

    pub async fn list_brands(&self) -> SResult<Vec<BrandSummary>> {
        let brands: Vec<(i64, String, String)> =
            sqlx::query_as("SELECT id, name, stock_mode FROM brands ORDER BY name")
                .fetch_all(&self.pool)
                .await?;
        let items = self.all_items().await?;
        let channels = self.channels().await?;
        Ok(brands
            .into_iter()
            .map(|(id, name, stock_mode)| {
                let mine: Vec<&ItemSummary> = items.iter().filter(|i| i.brand == name).collect();
                BrandSummary {
                    id,
                    item_count: mine.len() as i64,
                    low_stock_count: mine.iter().filter(|i| i.status == "low").count() as i64,
                    out_of_stock_count: mine.iter().filter(|i| i.status == "out").count() as i64,
                    stock_value: stock_value(&mine),
                    stock_mode,
                    channels: channels
                        .iter()
                        .filter(|(b, _)| *b == id)
                        .map(|(_, c)| c.clone())
                        .collect(),
                    name,
                }
            })
            .collect())
    }

    pub async fn sales_summary(&self, days: i64, brand: Option<&str>) -> SResult<SalesSummary> {
        let days = days.clamp(1, 365);
        let since = days_ago(days);
        let base = "FROM order_lines o JOIN listings l ON l.id = o.listing_id JOIN items i ON i.id = l.item_id JOIN brands b ON b.id = i.brand_id
                    WHERE o.ordered_at >= ?1 AND (?2 IS NULL OR b.name = ?2 COLLATE NOCASE)";
        let (units, revenue, orders): (Option<i64>, Option<f64>, i64) = sqlx::query_as(&format!(
            "SELECT SUM(o.quantity), SUM(o.quantity * COALESCE(o.unit_price, 0)), COUNT(DISTINCT o.channel_id || ':' || o.external_order_id) {base}"
        ))
        .bind(&since)
        .bind(brand)
        .fetch_one(&self.pool)
        .await?;
        let by_day = sqlx::query_as(&format!(
            "SELECT substr(o.ordered_at, 1, 10) AS day, SUM(o.quantity) AS units {base} GROUP BY day ORDER BY day"
        ))
        .bind(&since)
        .bind(brand)
        .fetch_all(&self.pool)
        .await?;
        let top_items = sqlx::query_as(&format!(
            "SELECT i.sku, i.name, b.name AS brand, SUM(o.quantity) AS units, SUM(o.quantity * COALESCE(o.unit_price, 0)) AS revenue {base}
             GROUP BY i.id ORDER BY units DESC LIMIT 10"
        ))
        .bind(&since)
        .bind(brand)
        .fetch_all(&self.pool)
        .await?;
        Ok(SalesSummary {
            period_days: days,
            brand: brand.map(Into::into),
            units: units.unwrap_or(0),
            revenue: (revenue.unwrap_or(0.0) * 100.0).round() / 100.0,
            orders,
            by_day,
            top_items,
        })
    }

    pub async fn overview(&self) -> SResult<Overview> {
        let items = self.all_items().await?;
        let brands = self.list_brands().await?;
        let sales = self.sales_summary(30, None).await?;
        let (pending,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM proposals WHERE status = 'pending'")
                .fetch_one(&self.pool)
                .await?;
        let mut attention: Vec<ItemSummary> =
            items.iter().filter(|i| i.status != "ok").cloned().collect();
        attention.sort_by(|a, b| {
            a.days_cover
                .unwrap_or(f64::MAX)
                .total_cmp(&b.days_cover.unwrap_or(f64::MAX))
                .then(a.on_hand.cmp(&b.on_hand))
        });
        attention.truncate(8);
        let all: Vec<&ItemSummary> = items.iter().collect();
        Ok(Overview {
            items: items.len() as i64,
            low_stock: items.iter().filter(|i| i.status == "low").count() as i64,
            out_of_stock: items.iter().filter(|i| i.status == "out").count() as i64,
            pending_proposals: pending,
            units_30d: sales.units,
            revenue_30d: sales.revenue,
            stock_value: stock_value(&all),
            attention,
            recent_activity: self.activity(10, None).await?,
            brands,
        })
    }

    pub async fn activity(
        &self,
        limit: i64,
        actor_kind: Option<&str>,
    ) -> SResult<Vec<ActivityEntry>> {
        Ok(sqlx::query_as(
            "SELECT id, actor_kind, actor, action, subject_type, subject_id, summary, created_at FROM activity
             WHERE (?1 IS NULL OR actor_kind = ?1) ORDER BY id DESC LIMIT ?2",
        )
        .bind(actor_kind)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn tags(&self) -> SResult<Vec<TagCount>> {
        Ok(sqlx::query_as(
            "SELECT t.name, COUNT(it.item_id) AS items FROM tags t LEFT JOIN item_tags it ON it.tag_id = t.id GROUP BY t.id ORDER BY t.name",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn suppliers(&self) -> SResult<Vec<Supplier>> {
        Ok(sqlx::query_as(
            "SELECT s.id, s.name, s.email, s.lead_time_days, s.notes, (SELECT COUNT(*) FROM items i WHERE i.supplier_id = s.id AND i.archived = 0) AS items
             FROM suppliers s ORDER BY s.name",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    async fn proposals_where(
        &self,
        cond: &str,
        arg: Option<i64>,
        limit: i64,
    ) -> SResult<Vec<ProposalView>> {
        let rows: Vec<ProposalRow> = sqlx::query_as(&format!(
            "{PROPOSAL_SELECT} WHERE {cond} ORDER BY p.id DESC LIMIT {limit}"
        ))
        .bind(arg)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn list_proposals(&self, status: Option<&str>) -> SResult<Vec<ProposalView>> {
        let rows: Vec<ProposalRow> = sqlx::query_as(&format!(
            "{PROPOSAL_SELECT} WHERE (?1 IS NULL OR p.status = ?1) ORDER BY p.status = 'pending' DESC, p.id DESC LIMIT 200"
        ))
        .bind(status.filter(|s| !s.is_empty() && *s != "all"))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    pub async fn get_proposal(&self, id: i64) -> SResult<ProposalView> {
        self.proposals_where("p.id = ?", Some(id), 1)
            .await?
            .pop()
            .ok_or_else(|| ServiceError::NotFound(format!("proposal {id} not found")))
    }

    // ---------- organise ----------

    async fn item_label(&self, id: i64) -> SResult<String> {
        let (sku,): (String,) = sqlx::query_as("SELECT sku FROM items WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(sku)
    }

    pub async fn tag_items(
        &self,
        who: &Principal,
        item_ids: &[i64],
        tags: &[String],
        remove: bool,
    ) -> SResult<()> {
        who.require(Scope::Organise)?;
        let tags: Vec<String> = tags
            .iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        if tags.is_empty() || item_ids.is_empty() {
            return invalid("need at least one item and one tag");
        }
        let mut tx = self.pool.begin().await?;
        for tag in &tags {
            sqlx::query("INSERT OR IGNORE INTO tags (name) VALUES (?)")
                .bind(tag)
                .execute(&mut *tx)
                .await?;
            let (tag_id,): (i64,) = sqlx::query_as("SELECT id FROM tags WHERE name = ?")
                .bind(tag)
                .fetch_one(&mut *tx)
                .await?;
            for id in item_ids {
                let sql = if remove {
                    "DELETE FROM item_tags WHERE item_id = ? AND tag_id = ?"
                } else {
                    "INSERT OR IGNORE INTO item_tags (item_id, tag_id) VALUES (?, ?)"
                };
                sqlx::query(sql)
                    .bind(id)
                    .bind(tag_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        let mut skus = Vec::new();
        for id in item_ids {
            skus.push(self.item_label(*id).await?);
        }
        let verb = if remove { "untagged" } else { "tagged" };
        let subject = (item_ids.len() == 1).then(|| ("item", item_ids[0]));
        self.log(
            who,
            if remove { "untag" } else { "tag" },
            subject,
            format!("{verb} {} with {}", skus.join(", "), tags.join(", ")),
        )
        .await
    }

    pub async fn set_supplier(
        &self,
        who: &Principal,
        item_ids: &[i64],
        supplier: Option<&str>,
        email: Option<&str>,
        lead_time_days: Option<i64>,
    ) -> SResult<()> {
        who.require(Scope::Organise)?;
        let supplier_id = match supplier.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(name) => {
                sqlx::query("INSERT OR IGNORE INTO suppliers (name) VALUES (?)")
                    .bind(name)
                    .execute(&self.pool)
                    .await?;
                sqlx::query(
                    "UPDATE suppliers SET email = COALESCE(?, email), lead_time_days = COALESCE(?, lead_time_days) WHERE name = ?",
                )
                .bind(email)
                .bind(lead_time_days)
                .bind(name)
                .execute(&self.pool)
                .await?;
                let (id,): (i64,) = sqlx::query_as("SELECT id FROM suppliers WHERE name = ?")
                    .bind(name)
                    .fetch_one(&self.pool)
                    .await?;
                Some(id)
            }
        };
        for id in item_ids {
            sqlx::query("UPDATE items SET supplier_id = ? WHERE id = ?")
                .bind(supplier_id)
                .bind(id)
                .execute(&self.pool)
                .await?;
            let sku = self.item_label(*id).await?;
            self.log(
                who,
                "set_supplier",
                Some(("item", *id)),
                format!("{sku} supplier → {}", supplier.unwrap_or("none")),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn update_item(&self, who: &Principal, id: i64, s: &ItemSettings) -> SResult<()> {
        who.require(Scope::Organise)?;
        for (name, v) in [
            ("reorder_point", s.reorder_point),
            ("target_stock", s.target_stock),
            ("lead_time_days", s.lead_time_days),
        ] {
            if v.is_some_and(|v| v < 0) {
                return invalid(format!("{name} must be >= 0"));
            }
        }
        sqlx::query(
            "UPDATE items SET reorder_point = COALESCE(?, reorder_point), target_stock = COALESCE(?, target_stock),
               lead_time_days = COALESCE(?, lead_time_days), unit_cost = COALESCE(?, unit_cost) WHERE id = ?",
        )
        .bind(s.reorder_point)
        .bind(s.target_stock)
        .bind(s.lead_time_days)
        .bind(s.unit_cost)
        .bind(id)
        .execute(&self.pool)
        .await?;
        let mut parts = Vec::new();
        if let Some(v) = s.reorder_point {
            parts.push(format!("reorder point {v}"));
        }
        if let Some(v) = s.target_stock {
            parts.push(format!("target stock {v}"));
        }
        if let Some(v) = s.lead_time_days {
            parts.push(format!("lead time {v}d"));
        }
        if let Some(v) = s.unit_cost {
            parts.push(format!("unit cost {v:.2}"));
        }
        if parts.is_empty() {
            return invalid("nothing to update");
        }
        let sku = self.item_label(id).await?;
        self.log(
            who,
            "update_item",
            Some(("item", id)),
            format!("{sku}: {}", parts.join(", ")),
        )
        .await
    }

    pub async fn add_note(&self, who: &Principal, id: i64, body: &str) -> SResult<()> {
        who.require(Scope::Organise)?;
        if body.trim().is_empty() {
            return invalid("note body is empty");
        }
        sqlx::query("INSERT INTO notes (item_id, body, actor) VALUES (?, ?, ?)")
            .bind(id)
            .bind(body.trim())
            .bind(&who.name)
            .execute(&self.pool)
            .await?;
        let sku = self.item_label(id).await?;
        let preview: String = body.trim().chars().take(80).collect();
        self.log(
            who,
            "note",
            Some(("item", id)),
            format!("note on {sku}: {preview}"),
        )
        .await
    }

    /// Move every listing of `source` onto `target` and archive `source`.
    pub async fn merge_items(&self, who: &Principal, source: i64, target: i64) -> SResult<()> {
        who.require(Scope::Organise)?;
        if source == target {
            return invalid("source and target are the same item");
        }
        let (sb, tb): ((i64,), (i64,)) = (
            sqlx::query_as("SELECT brand_id FROM items WHERE id = ?")
                .bind(source)
                .fetch_one(&self.pool)
                .await?,
            sqlx::query_as("SELECT brand_id FROM items WHERE id = ?")
                .bind(target)
                .fetch_one(&self.pool)
                .await?,
        );
        if sb != tb {
            return invalid("can only merge items of the same brand");
        }
        let master = self.item_stock_mode(target).await? == "master";
        let mut tx = self.pool.begin().await?;
        if master {
            crate::ledger::merge_ledgers(&mut tx, source, target, &who.name).await?;
        }
        sqlx::query("UPDATE listings SET item_id = ? WHERE item_id = ?")
            .bind(target)
            .bind(source)
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO item_tags (item_id, tag_id) SELECT ?, tag_id FROM item_tags WHERE item_id = ?").bind(target).bind(source).execute(&mut *tx).await?;
        sqlx::query("UPDATE notes SET item_id = ? WHERE item_id = ?")
            .bind(target)
            .bind(source)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE items SET archived = 1 WHERE id = ?")
            .bind(source)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        if master {
            self.push_stock(sb.0, Some(target), None).await?;
        }
        let (s, t) = (
            self.item_label(source).await?,
            self.item_label(target).await?,
        );
        self.log(
            who,
            "merge",
            Some(("item", target)),
            format!("merged {s} into {t}"),
        )
        .await
    }

    // ---------- proposals ----------

    async fn pick_listing(
        &self,
        item_id: i64,
        channel: Option<&str>,
    ) -> SResult<(i64, String, Option<f64>, Option<String>)> {
        let rows: Vec<(i64, String, Option<f64>, Option<String>)> = sqlx::query_as(
            "SELECT l.id, c.name, l.price, l.status FROM listings l JOIN channels c ON c.id = l.channel_id
             WHERE l.item_id = ?1 AND (?2 IS NULL OR c.name = ?2 COLLATE NOCASE OR c.kind = ?2 COLLATE NOCASE)",
        )
        .bind(item_id)
        .bind(channel)
        .fetch_all(&self.pool)
        .await?;
        match rows.len() {
            0 => Err(ServiceError::NotFound(match channel {
                Some(c) => format!("item has no listing on channel `{c}`"),
                None => "item has no channel listings".into(),
            })),
            1 => Ok(rows.into_iter().next().unwrap()),
            _ => invalid(format!(
                "item is listed on several channels ({}); pass `channel`",
                rows.iter()
                    .map(|r| r.1.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn insert_proposal(
        &self,
        who: &Principal,
        kind: &str,
        item_id: i64,
        listing_id: Option<i64>,
        before: Value,
        after: Value,
        rationale: Option<&str>,
    ) -> SResult<ProposalView> {
        let dupe: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM proposals WHERE status = 'pending' AND kind = ? AND item_id = ? AND listing_id IS ? AND after_json = ?",
        )
        .bind(kind)
        .bind(item_id)
        .bind(listing_id)
        .bind(after.to_string())
        .fetch_optional(&self.pool)
        .await?;
        if let Some((id,)) = dupe {
            return Err(ServiceError::Conflict(format!(
                "an identical pending proposal already exists (#{id})"
            )));
        }
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO proposals (kind, item_id, listing_id, before_json, after_json, rationale, actor) VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(kind)
        .bind(item_id)
        .bind(listing_id)
        .bind(before.to_string())
        .bind(after.to_string())
        .bind(rationale)
        .bind(&who.name)
        .fetch_one(&self.pool)
        .await?;
        let p = self.get_proposal(id).await?;
        self.log(
            who,
            "propose",
            Some(("proposal", id)),
            format!(
                "proposed {} for {}: {}",
                kind.replace('_', " "),
                p.item_sku.clone().unwrap_or_default(),
                describe(&p)
            ),
        )
        .await?;
        Ok(p)
    }

    pub async fn propose_stock(&self, who: &Principal, p: &ProposeStock) -> SResult<ProposalView> {
        who.require(Scope::Propose)?;
        let item_id = self.resolve_item(&p.item, p.brand.as_deref()).await?;
        if self.item_stock_mode(item_id).await? == "master" {
            return self.propose_ledger_stock(who, item_id, p).await;
        }
        let (listing_id, _, _, _) = self.pick_listing(item_id, p.channel.as_deref()).await?;
        let rows: Vec<(i64, String, i64)> = sqlx::query_as(
            "SELECT lo.id, lo.name, ls.quantity FROM listing_stock ls JOIN locations lo ON lo.id = ls.location_id
             WHERE ls.listing_id = ?1 AND (?2 IS NULL OR lo.name = ?2 COLLATE NOCASE)",
        )
        .bind(listing_id)
        .bind(p.location.as_deref())
        .fetch_all(&self.pool)
        .await?;
        let (loc_id, loc_name, current) = match rows.len() {
            0 => {
                return Err(ServiceError::NotFound(
                    "listing has no stock at that location".into(),
                ));
            }
            1 => rows.into_iter().next().unwrap(),
            _ => {
                return invalid(format!(
                    "listing has stock at several locations ({}); pass `location`",
                    rows.iter()
                        .map(|r| r.1.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        };
        let quantity = match (p.quantity, p.delta) {
            (Some(q), None) => q,
            (None, Some(d)) => current + d,
            _ => return invalid("pass exactly one of `quantity` or `delta`"),
        };
        if quantity < 0 {
            return invalid("stock cannot go below 0");
        }
        if quantity == current {
            return invalid(format!("stock is already {current}"));
        }
        self.insert_proposal(
            who,
            "stock_adjustment",
            item_id,
            Some(listing_id),
            json!({ "location_id": loc_id, "location": loc_name, "quantity": current }),
            json!({ "location_id": loc_id, "location": loc_name, "quantity": quantity }),
            p.rationale.as_deref(),
        )
        .await
    }

    pub async fn propose_price(&self, who: &Principal, p: &ProposePrice) -> SResult<ProposalView> {
        who.require(Scope::Propose)?;
        if p.price.is_nan() || p.price <= 0.0 {
            return invalid("price must be > 0");
        }
        let item_id = self.resolve_item(&p.item, p.brand.as_deref()).await?;
        let (listing_id, _, price, _) = self.pick_listing(item_id, p.channel.as_deref()).await?;
        let new = (p.price * 100.0).round() / 100.0;
        if price.is_some_and(|c| (c - new).abs() < 0.005) {
            return invalid(format!("price is already {new:.2}"));
        }
        self.insert_proposal(
            who,
            "price_change",
            item_id,
            Some(listing_id),
            json!({ "price": price }),
            json!({ "price": new }),
            p.rationale.as_deref(),
        )
        .await
    }

    pub async fn propose_status(
        &self,
        who: &Principal,
        p: &ProposeStatus,
    ) -> SResult<ProposalView> {
        who.require(Scope::Propose)?;
        let status = p.status.trim().to_lowercase();
        if !["active", "draft", "archived"].contains(&status.as_str()) {
            return invalid("status must be active, draft or archived");
        }
        let item_id = self.resolve_item(&p.item, p.brand.as_deref()).await?;
        let (listing_id, _, _, current) = self.pick_listing(item_id, p.channel.as_deref()).await?;
        if current.as_deref() == Some(status.as_str()) {
            return invalid(format!("listing is already {status}"));
        }
        self.insert_proposal(
            who,
            "listing_status",
            item_id,
            Some(listing_id),
            json!({ "status": current }),
            json!({ "status": status }),
            p.rationale.as_deref(),
        )
        .await
    }

    pub async fn reject(
        &self,
        who: &Principal,
        id: i64,
        reason: Option<&str>,
    ) -> SResult<ProposalView> {
        who.require(Scope::Approve)?;
        let p = self.get_proposal(id).await?;
        if p.status != "pending" {
            return Err(ServiceError::Conflict(format!(
                "proposal #{id} is already {}",
                p.status
            )));
        }
        let claimed = sqlx::query("UPDATE proposals SET status = 'rejected', decided_by = ?, decided_at = ?, error = ? WHERE id = ? AND status = 'pending'")
            .bind(&who.name)
            .bind(now())
            .bind(reason)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if claimed.rows_affected() == 0 {
            return Err(ServiceError::Conflict(format!(
                "proposal #{id} was decided concurrently"
            )));
        }
        self.log(
            who,
            "reject",
            Some(("proposal", id)),
            format!(
                "rejected #{id} ({}){}",
                describe(&p),
                reason.map(|r| format!(": {r}")).unwrap_or_default()
            ),
        )
        .await?;
        self.get_proposal(id).await
    }

    /// Push an approved proposal to its channel and mirror the result locally.
    pub async fn approve(&self, who: &Principal, id: i64) -> SResult<ProposalView> {
        who.require(Scope::Approve)?;
        if who.kind != "human" && who.kind != "system" {
            return Err(ServiceError::Forbidden(
                "only human tokens can approve proposals".into(),
            ));
        }
        let p = self.get_proposal(id).await?;
        if p.status != "pending" {
            return Err(ServiceError::Conflict(format!(
                "proposal #{id} is already {}",
                p.status
            )));
        }
        let claimed = sqlx::query("UPDATE proposals SET status = 'applied', decided_by = ?, decided_at = ? WHERE id = ? AND status = 'pending'")
            .bind(&who.name)
            .bind(now())
            .bind(id)
            .execute(&self.pool)
            .await?;
        if claimed.rows_affected() == 0 {
            return Err(ServiceError::Conflict(format!(
                "proposal #{id} was decided concurrently"
            )));
        }
        match self.apply(&p).await {
            Ok(()) => {
                self.log(
                    who,
                    "approve",
                    Some(("proposal", id)),
                    format!("approved #{id}: {} on {}", describe(&p), p.channel),
                )
                .await?;
            }
            Err(e) => {
                let msg = format!("{e:#}");
                sqlx::query("UPDATE proposals SET status = 'failed', error = ? WHERE id = ?")
                    .bind(&msg)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                self.log(
                    who,
                    "apply_failed",
                    Some(("proposal", id)),
                    format!("#{id} failed on {}: {msg}", p.channel),
                )
                .await?;
            }
        }
        self.get_proposal(id).await
    }

    async fn apply(&self, p: &ProposalView) -> anyhow::Result<()> {
        if self.apply_ledger(p).await? {
            return Ok(());
        }
        let listing_id = p.listing_id.context("proposal has no listing")?;
        #[derive(FromRow)]
        struct Row {
            kind: String,
            base_url: String,
            credential_env: String,
            external_product_id: String,
            external_variant_id: String,
            inventory_ref: Option<String>,
        }
        let r: Row = sqlx::query_as(
            "SELECT c.kind, c.base_url, c.credential_env, l.external_product_id, l.external_variant_id, l.inventory_ref
             FROM listings l JOIN channels c ON c.id = l.channel_id WHERE l.id = ?",
        )
        .bind(listing_id)
        .fetch_one(&self.pool)
        .await?;
        let credential = std::env::var(&r.credential_env)
            .map_err(|_| anyhow::anyhow!("env var {} is not set", r.credential_env))?;
        let conn = connectors::build(&r.kind, &r.base_url, credential)?;
        let target = ListingRef {
            external_product_id: r.external_product_id,
            external_variant_id: r.external_variant_id,
            inventory_ref: r.inventory_ref,
        };
        match p.kind.as_str() {
            "stock_adjustment" => {
                let loc_id = p.after["location_id"].as_i64().unwrap_or_default();
                let qty = p.after["quantity"].as_i64().unwrap_or_default();
                let before = p.before["quantity"].as_i64().unwrap_or_default();
                let (ext, stock_ref): (String, Option<String>) = sqlx::query_as(
                    "SELECT lo.external_id, ls.stock_ref FROM listing_stock ls JOIN locations lo ON lo.id = ls.location_id WHERE ls.listing_id = ? AND ls.location_id = ?",
                )
                .bind(listing_id)
                .bind(loc_id)
                .fetch_one(&self.pool)
                .await?;
                conn.set_stock(&target, &ext, stock_ref.as_deref(), before, qty)
                    .await?;
                sqlx::query("UPDATE listing_stock SET quantity = ?, updated_at = ? WHERE listing_id = ? AND location_id = ?")
                    .bind(qty)
                    .bind(now())
                    .bind(listing_id)
                    .bind(loc_id)
                    .execute(&self.pool)
                    .await?;
                self.snapshot(listing_id).await?;
            }
            "price_change" => {
                let price = p.after["price"].as_f64().unwrap_or_default();
                conn.set_price(&target, price).await?;
                sqlx::query("UPDATE listings SET price = ? WHERE id = ?")
                    .bind(price)
                    .bind(listing_id)
                    .execute(&self.pool)
                    .await?;
            }
            "listing_status" => {
                let status = p.after["status"].as_str().unwrap_or_default();
                conn.set_status(&target, status).await?;
                let local = if r.kind == "prestashop" && status != "active" {
                    "draft"
                } else {
                    status
                };
                // Status is product-level upstream, so every variant listing of the product changes.
                sqlx::query(
                    "UPDATE listings SET status = ? WHERE (channel_id, external_product_id) =
                       (SELECT channel_id, external_product_id FROM listings WHERE id = ?)",
                )
                .bind(local)
                .bind(listing_id)
                .execute(&self.pool)
                .await?;
            }
            other => anyhow::bail!("unknown proposal kind {other}"),
        }
        Ok(())
    }

    async fn snapshot(&self, listing_id: i64) -> SResult<()> {
        sqlx::query("INSERT INTO stock_snapshots (listing_id, quantity) SELECT listing_id, SUM(quantity) FROM listing_stock WHERE listing_id = ? GROUP BY listing_id")
            .bind(listing_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---------- channels & sync ----------

    pub async fn add_channel(&self, who: &Principal, c: &NewChannel) -> SResult<i64> {
        who.require(Scope::Admin)?;
        if c.kind != "shopify" && c.kind != "prestashop" {
            return invalid("kind must be shopify or prestashop");
        }
        sqlx::query("INSERT OR IGNORE INTO brands (name) VALUES (?)")
            .bind(c.brand.trim())
            .execute(&self.pool)
            .await?;
        let (brand_id,): (i64,) = sqlx::query_as("SELECT id FROM brands WHERE name = ?")
            .bind(c.brand.trim())
            .fetch_one(&self.pool)
            .await?;
        let existing: Option<(String, String, i64)> = sqlx::query_as(
            "SELECT c.kind, c.base_url, (SELECT COUNT(*) FROM listings l WHERE l.channel_id = c.id)
             FROM channels c WHERE c.brand_id = ? AND c.name = ?",
        )
        .bind(brand_id)
        .bind(c.name.trim())
        .fetch_optional(&self.pool)
        .await?;
        if let Some((kind, url, listings)) = existing
            && listings > 0
            && (kind != c.kind || url != c.base_url.trim())
        {
            return invalid(
                "channel already has listings from another store; use a new channel name",
            );
        }
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO channels (brand_id, kind, name, base_url, credential_env) VALUES (?, ?, ?, ?, ?)
             ON CONFLICT (brand_id, name) DO UPDATE SET kind = excluded.kind, base_url = excluded.base_url, credential_env = excluded.credential_env
             RETURNING id",
        )
        .bind(brand_id)
        .bind(&c.kind)
        .bind(c.name.trim())
        .bind(c.base_url.trim())
        .bind(c.credential_env.trim())
        .fetch_one(&self.pool)
        .await?;
        self.log(
            who,
            "add_channel",
            Some(("channel", id)),
            format!("connected {} channel {} for {}", c.kind, c.name, c.brand),
        )
        .await?;
        Ok(id)
    }

    pub async fn sync_all(&self, who: &Principal) -> SResult<Vec<SyncReport>> {
        let ids: Vec<(i64,)> = sqlx::query_as("SELECT id FROM channels ORDER BY id")
            .fetch_all(&self.pool)
            .await?;
        let mut out = Vec::new();
        for (id,) in ids {
            match self.sync_channel(who, id).await {
                Ok(r) => out.push(r),
                Err(e) => tracing::warn!(channel = id, error = %e, "sync failed"),
            }
        }
        Ok(out)
    }

    pub async fn sync_channel(&self, who: &Principal, channel_id: i64) -> SResult<SyncReport> {
        who.require(Scope::Read)?;
        let res = self.sync_inner(who, channel_id).await;
        let err = res.as_ref().err().map(|e| format!("{e:#}"));
        sqlx::query("UPDATE channels SET last_sync_error = ?, last_synced_at = CASE WHEN ? IS NULL THEN ? ELSE last_synced_at END WHERE id = ?")
            .bind(&err)
            .bind(&err)
            .bind(now())
            .bind(channel_id)
            .execute(&self.pool)
            .await?;
        match res {
            Ok(mut r) => {
                r.master = self.reconcile(channel_id).await?;
                let master = r
                    .master
                    .as_ref()
                    .map(|m| {
                        format!(
                            "; ledger: {} sold, {} pushed, {} drift, {} oversold",
                            m.units_sold, m.pushed, m.drift, m.oversold
                        )
                    })
                    .unwrap_or_default();
                self.log(
                    who,
                    "sync",
                    Some(("channel", channel_id)),
                    format!(
                        "synced {}: {} listings, {} new items, {} order lines{master}",
                        r.channel, r.listings, r.new_items, r.order_lines
                    ),
                )
                .await?;
                Ok(r)
            }
            Err(e) => {
                self.log(
                    who,
                    "sync_failed",
                    Some(("channel", channel_id)),
                    format!("sync of channel {channel_id} failed: {e:#}"),
                )
                .await?;
                Err(ServiceError::Internal(e))
            }
        }
    }

    async fn sync_inner(&self, _who: &Principal, channel_id: i64) -> anyhow::Result<SyncReport> {
        let (brand_id, kind, name, base_url, cred_env): (i64, String, String, String, String) =
            sqlx::query_as(
                "SELECT brand_id, kind, name, base_url, credential_env FROM channels WHERE id = ?",
            )
            .bind(channel_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("channel {channel_id} not found"))?;
        let credential = std::env::var(&cred_env)
            .map_err(|_| anyhow::anyhow!("env var {cred_env} is not set"))?;
        let conn = connectors::build(&kind, &base_url, credential)?;
        let catalog = conn.fetch_catalog().await?;
        let orders = conn.fetch_orders(&days_ago(90)).await?;

        let mut tx = self.pool.begin().await?;
        let mut loc_ids = HashMap::new();
        for l in &catalog.locations {
            let (id,): (i64,) = sqlx::query_as(
                "INSERT INTO locations (channel_id, external_id, name) VALUES (?, ?, ?)
                 ON CONFLICT (channel_id, external_id) DO UPDATE SET name = excluded.name RETURNING id",
            )
            .bind(channel_id)
            .bind(&l.external_id)
            .bind(&l.name)
            .fetch_one(&mut *tx)
            .await?;
            loc_ids.insert(l.external_id.clone(), id);
        }
        let mut new_items = 0;
        let mut listing_ids = HashMap::new();
        let ts = now();
        for l in &catalog.listings {
            let sku = l.sku.clone().unwrap_or_else(|| {
                let short = |s: &str| s.rsplit('/').next().unwrap_or(s).to_owned();
                format!(
                    "{kind}-{}{}",
                    short(&l.external_product_id),
                    if l.external_variant_id.is_empty() {
                        String::new()
                    } else {
                        format!("-{}", short(&l.external_variant_id))
                    }
                )
            });
            let existing: Option<(i64,)> = sqlx::query_as("SELECT item_id FROM listings WHERE channel_id = ? AND external_product_id = ? AND external_variant_id = ? AND item_id IS NOT NULL")
                .bind(channel_id)
                .bind(&l.external_product_id)
                .bind(&l.external_variant_id)
                .fetch_optional(&mut *tx)
                .await?;
            let item_id = match existing {
                Some((id,)) => id,
                None => {
                    let res = sqlx::query(
                        "INSERT OR IGNORE INTO items (brand_id, sku, name) VALUES (?, ?, ?)",
                    )
                    .bind(brand_id)
                    .bind(&sku)
                    .bind(&l.title)
                    .execute(&mut *tx)
                    .await?;
                    new_items += res.rows_affected() as usize;
                    let (id,): (i64,) =
                        sqlx::query_as("SELECT id FROM items WHERE brand_id = ? AND sku = ?")
                            .bind(brand_id)
                            .bind(&sku)
                            .fetch_one(&mut *tx)
                            .await?;
                    id
                }
            };
            let (listing_id,): (i64,) = sqlx::query_as(
                "INSERT INTO listings (channel_id, item_id, external_product_id, external_variant_id, inventory_ref, sku, title, price, status, image_url, synced_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (channel_id, external_product_id, external_variant_id) DO UPDATE SET
                   inventory_ref = excluded.inventory_ref, sku = excluded.sku, title = excluded.title, price = excluded.price,
                   status = excluded.status, image_url = excluded.image_url, synced_at = excluded.synced_at,
                   item_id = COALESCE(listings.item_id, excluded.item_id)
                 RETURNING id",
            )
            .bind(channel_id)
            .bind(item_id)
            .bind(&l.external_product_id)
            .bind(&l.external_variant_id)
            .bind(&l.inventory_ref)
            .bind(&l.sku)
            .bind(&l.title)
            .bind(l.price)
            .bind(&l.status)
            .bind(&l.image_url)
            .bind(&ts)
            .fetch_one(&mut *tx)
            .await?;
            listing_ids.insert(
                (l.external_product_id.clone(), l.external_variant_id.clone()),
                listing_id,
            );
            let mut total = 0;
            for s in &l.stock {
                let Some(loc) = loc_ids.get(&s.location_external_id) else {
                    continue;
                };
                total += s.quantity;
                sqlx::query(
                    "INSERT INTO listing_stock (listing_id, location_id, stock_ref, quantity, updated_at) VALUES (?, ?, ?, ?, ?)
                     ON CONFLICT (listing_id, location_id) DO UPDATE SET stock_ref = excluded.stock_ref, quantity = excluded.quantity, updated_at = excluded.updated_at",
                )
                .bind(listing_id)
                .bind(loc)
                .bind(&s.stock_ref)
                .bind(s.quantity)
                .bind(&ts)
                .execute(&mut *tx)
                .await?;
            }
            sqlx::query(
                "INSERT INTO stock_snapshots (listing_id, quantity, taken_at) VALUES (?, ?, ?)",
            )
            .bind(listing_id)
            .bind(total)
            .bind(&ts)
            .execute(&mut *tx)
            .await?;
        }
        // Anything not returned by this sync is gone upstream: drop its stock so totals stay honest.
        sqlx::query(
            "DELETE FROM listing_stock WHERE updated_at <> ? AND listing_id IN (SELECT id FROM listings WHERE channel_id = ?)",
        )
        .bind(&ts)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE listings SET status = 'removed' WHERE channel_id = ? AND synced_at <> ?",
        )
        .bind(channel_id)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
        for o in &orders {
            let listing =
                listing_ids.get(&(o.external_product_id.clone(), o.external_variant_id.clone()));
            sqlx::query(
                "INSERT INTO order_lines (channel_id, external_order_id, external_line_id, listing_id, sku, quantity, unit_price, ordered_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT (channel_id, external_order_id, external_line_id) DO UPDATE SET listing_id = excluded.listing_id, quantity = excluded.quantity",
            )
            .bind(channel_id)
            .bind(&o.external_order_id)
            .bind(&o.external_line_id)
            .bind(listing)
            .bind(&o.sku)
            .bind(o.quantity)
            .bind(o.unit_price)
            .bind(&o.ordered_at)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(SyncReport {
            channel_id,
            channel: name,
            listings: catalog.listings.len(),
            new_items,
            order_lines: orders.len(),
            master: None,
        })
    }
}

fn stock_value(items: &[&ItemSummary]) -> f64 {
    let v: f64 = items
        .iter()
        .map(|i| i.on_hand.max(0) as f64 * i.unit_cost.or(i.price).unwrap_or(0.0))
        .sum();
    (v * 100.0).round() / 100.0
}

pub fn describe(p: &ProposalView) -> String {
    match p.kind.as_str() {
        "stock_transfer" => format!(
            "move {} from {} to {}",
            p.after["quantity"],
            p.after["from"].as_str().unwrap_or("?"),
            p.after["to"].as_str().unwrap_or("?")
        ),
        "stock_adjustment" | "ledger_adjustment" => format!(
            "stock {} → {} at {}",
            p.before["quantity"],
            p.after["quantity"],
            p.after["location"].as_str().unwrap_or("?")
        ),
        "price_change" => format!("price {} → {}", p.before["price"], p.after["price"]),
        "listing_status" => format!(
            "status {} → {}",
            p.before["status"].as_str().unwrap_or("?"),
            p.after["status"].as_str().unwrap_or("?")
        ),
        k => k.to_owned(),
    }
}
