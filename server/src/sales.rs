//! Sales and wholesale orders that arrive outside the shops (email, phone, a photo of an order
//! sheet). The agent reads them and proposes a sales order; a human confirms it, which reserves
//! stock (master brands push the reduced availability to every channel), then fulfils it, which
//! records the sale in the ledger.

use crate::auth::{Principal, Scope};
use crate::service::{SResult, Service, ServiceError, invalid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct SoLine {
    pub id: i64,
    pub item_id: i64,
    pub sku: String,
    pub name: String,
    pub quantity: i64,
    pub unit_price: Option<f64>,
    /// Current on-hand minus other confirmed orders; below `quantity` means a shortfall.
    pub available: i64,
}

#[derive(Debug, Serialize)]
pub struct SalesOrder {
    pub id: i64,
    pub brand: String,
    pub customer: String,
    pub customer_email: Option<String>,
    pub external_ref: Option<String>,
    pub source: Option<String>,
    pub warehouse: Option<String>,
    /// `proposed` → `confirmed` → `fulfilled`, or `rejected` / `cancelled`.
    pub status: String,
    pub note: Option<String>,
    pub created_by: String,
    pub approved_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub units: i64,
    pub total: f64,
    pub warnings: Vec<String>,
    pub lines: Vec<SoLine>,
}

#[derive(sqlx::FromRow)]
struct SoRow {
    id: i64,
    brand_id: i64,
    brand: String,
    stock_mode: String,
    customer: String,
    customer_email: Option<String>,
    external_ref: Option<String>,
    source: Option<String>,
    warehouse_id: Option<i64>,
    warehouse: Option<String>,
    status: String,
    note: Option<String>,
    created_by: String,
    approved_by: Option<String>,
    created_at: String,
    updated_at: String,
}

const SO_SELECT: &str = "SELECT so.id, so.brand_id, b.name AS brand, b.stock_mode, so.customer, so.customer_email,
  so.external_ref, so.source, so.warehouse_id, w.name AS warehouse, so.status, so.note, so.created_by,
  so.approved_by, so.created_at, so.updated_at
FROM sales_orders so JOIN brands b ON b.id = so.brand_id LEFT JOIN warehouses w ON w.id = so.warehouse_id";

/// On-hand for an item minus confirmed orders other than `?1`. Master brands count only the
/// order's warehouse `?2` (when set); mirror brands count store stock overall.
const AVAILABLE: &str = "(CASE WHEN b.stock_mode = 'master'
    THEN COALESCE((SELECT SUM(s.delta) FROM stock_ledger s WHERE s.item_id = i.id AND (?2 IS NULL OR s.warehouse_id = ?2)), 0)
    ELSE COALESCE((SELECT SUM(ls.quantity) FROM listing_stock ls JOIN listings l ON l.id = ls.listing_id WHERE l.item_id = i.id), 0)
  END - COALESCE((SELECT SUM(x.quantity) FROM so_lines x JOIN sales_orders xo ON xo.id = x.so_id
    WHERE x.item_id = i.id AND xo.status = 'confirmed' AND xo.id <> ?1
      AND (?2 IS NULL OR b.stock_mode <> 'master' OR xo.warehouse_id = ?2)), 0))";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SoLineInput {
    /// Item id or SKU.
    pub item: String,
    pub quantity: i64,
    /// Agreed unit price; defaults to the item's lowest channel price.
    pub unit_price: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeSalesOrder {
    /// Brand name or id.
    pub brand: String,
    /// Customer / retailer name.
    pub customer: String,
    pub customer_email: Option<String>,
    /// The customer's PO or reference number.
    pub external_ref: Option<String>,
    /// Where the order came from, e.g. "email from buyer@shop.com, 3 Oct" or "photo of order sheet".
    pub source: Option<String>,
    pub lines: Vec<SoLineInput>,
    /// Warehouse to ship from (master brands). Defaults to the brand's first warehouse.
    pub warehouse: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SoListArgs {
    /// `open` (default: proposed + confirmed), `all`, or one status.
    pub status: Option<String>,
    pub brand: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SoIdArgs {
    pub id: i64,
}

fn human(who: &Principal, what: &str) -> SResult<()> {
    if who.kind != "human" && who.kind != "system" {
        return Err(ServiceError::Forbidden(format!(
            "only human tokens can {what}"
        )));
    }
    Ok(())
}

impl Service {
    async fn so_view(&self, r: SoRow) -> SResult<SalesOrder> {
        let lines: Vec<SoLine> = sqlx::query_as(&format!(
            "SELECT sl.id, sl.item_id, i.sku, i.name, sl.quantity, sl.unit_price, {AVAILABLE} AS available
             FROM so_lines sl JOIN items i ON i.id = sl.item_id JOIN brands b ON b.id = i.brand_id
             WHERE sl.so_id = ?1 ORDER BY i.sku"
        ))
        .bind(r.id)
        .bind(r.warehouse_id)
        .fetch_all(&self.pool)
        .await?;
        let open = r.status == "proposed" || r.status == "confirmed";
        let warnings = lines
            .iter()
            .filter(|l| open && l.quantity > l.available)
            .map(|l| {
                format!(
                    "{}: {} ordered, {} available",
                    l.sku,
                    l.quantity,
                    l.available.max(0)
                )
            })
            .collect();
        Ok(SalesOrder {
            id: r.id,
            brand: r.brand,
            customer: r.customer,
            customer_email: r.customer_email,
            external_ref: r.external_ref,
            source: r.source,
            warehouse: r.warehouse,
            status: r.status,
            note: r.note,
            created_by: r.created_by,
            approved_by: r.approved_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
            units: lines.iter().map(|l| l.quantity).sum(),
            total: lines
                .iter()
                .map(|l| l.quantity as f64 * l.unit_price.unwrap_or(0.0))
                .sum(),
            warnings,
            lines,
        })
    }

    async fn so_row(&self, id: i64) -> SResult<SoRow> {
        sqlx::query_as(&format!("{SO_SELECT} WHERE so.id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or_else(|| ServiceError::NotFound(format!("sales order #{id} not found")))
    }

    async fn so(&self, id: i64) -> SResult<SalesOrder> {
        let r = self.so_row(id).await?;
        self.so_view(r).await
    }

    pub async fn sales_orders(
        &self,
        who: &Principal,
        status: Option<&str>,
        brand: Option<&str>,
    ) -> SResult<Vec<SalesOrder>> {
        who.require(Scope::Read)?;
        let rows: Vec<SoRow> = sqlx::query_as(&format!(
            "{SO_SELECT} WHERE (?1 IS NULL OR so.status = ?1 OR (?1 = 'open' AND so.status IN ('proposed','confirmed')))
               AND (?2 IS NULL OR b.name = ?2 COLLATE NOCASE) ORDER BY so.id DESC"
        ))
        .bind(status.filter(|s| !s.is_empty() && *s != "all"))
        .bind(brand)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(self.so_view(r).await?);
        }
        Ok(out)
    }

    pub async fn sales_order(&self, who: &Principal, id: i64) -> SResult<SalesOrder> {
        who.require(Scope::Read)?;
        self.so(id).await
    }

    pub async fn propose_sales_order(
        &self,
        who: &Principal,
        p: &ProposeSalesOrder,
    ) -> SResult<SalesOrder> {
        who.require(Scope::Propose)?;
        if p.customer.trim().is_empty() {
            return invalid("customer is required");
        }
        if p.lines.is_empty() {
            return invalid("a sales order needs at least one line");
        }
        let brand: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, name FROM brands WHERE CAST(id AS TEXT) = ?1 OR name = ?1 COLLATE NOCASE",
        )
        .bind(p.brand.trim())
        .fetch_optional(&self.pool)
        .await?;
        let Some((brand_id, brand)) = brand else {
            return Err(ServiceError::NotFound(format!(
                "brand `{}` not found",
                p.brand
            )));
        };
        let mut lines: Vec<(i64, i64, Option<f64>)> = Vec::new();
        for l in &p.lines {
            if l.quantity <= 0 {
                return invalid(format!("{}: quantity must be positive", l.item));
            }
            if l.unit_price.is_some_and(|c| !c.is_finite() || c < 0.0) {
                return invalid(format!("{}: unit_price must be ≥ 0", l.item));
            }
            let id = self.resolve_item(&l.item, Some(&brand)).await?;
            let (item_brand, price): (i64, Option<f64>) = sqlx::query_as(
                "SELECT i.brand_id, (SELECT MIN(price) FROM listings WHERE item_id = i.id) FROM items i WHERE i.id = ?",
            )
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
            if item_brand != brand_id {
                return invalid(format!("{} is not a {brand} item", l.item));
            }
            match lines.iter_mut().find(|x| x.0 == id) {
                Some(x) => x.1 += l.quantity,
                None => lines.push((id, l.quantity, l.unit_price.or(price))),
            }
        }
        let warehouse: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM warehouses WHERE brand_id = ?1 AND (?2 IS NULL OR name = ?2 COLLATE NOCASE) ORDER BY id LIMIT 1",
        )
        .bind(brand_id)
        .bind(p.warehouse.as_deref())
        .fetch_optional(&self.pool)
        .await?;
        if let (Some(w), None) = (p.warehouse.as_deref(), warehouse) {
            return Err(ServiceError::NotFound(format!("warehouse `{w}` not found")));
        }
        let mut tx = self.pool.begin().await?;
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO sales_orders (brand_id, customer, customer_email, external_ref, source, warehouse_id, note, created_by)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(brand_id)
        .bind(p.customer.trim())
        .bind(p.customer_email.as_deref())
        .bind(p.external_ref.as_deref())
        .bind(p.source.as_deref())
        .bind(warehouse.map(|w| w.0))
        .bind(p.note.as_deref())
        .bind(&who.name)
        .fetch_one(&mut *tx)
        .await?;
        for (item, qty, price) in &lines {
            sqlx::query(
                "INSERT INTO so_lines (so_id, item_id, quantity, unit_price) VALUES (?, ?, ?, ?)",
            )
            .bind(id)
            .bind(item)
            .bind(qty)
            .bind(price)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        let units: i64 = lines.iter().map(|l| l.1).sum();
        self.log(
            who,
            "propose_so",
            Some(("sales_order", id)),
            format!(
                "proposed SO #{id} for {}: {} lines, {units} units",
                p.customer.trim(),
                lines.len()
            ),
        )
        .await?;
        self.so(id).await
    }

    async fn so_transition(
        &self,
        who: &Principal,
        id: i64,
        from: &str,
        to: &str,
        done: &str,
    ) -> SResult<SoRow> {
        let row = self.so_row(id).await?;
        let r = sqlx::query(
            "UPDATE sales_orders SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'),
               approved_by = CASE WHEN ?1 = 'confirmed' THEN ?2 ELSE approved_by END
             WHERE id = ?3 AND status = ?4",
        )
        .bind(to)
        .bind(&who.name)
        .bind(id)
        .bind(from)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(ServiceError::Conflict(format!(
                "SO #{id} is {}, not {from}",
                row.status
            )));
        }
        self.log(
            who,
            &format!("{to}_so"),
            Some(("sales_order", id)),
            format!("{done} SO #{id} for {}", row.customer),
        )
        .await?;
        Ok(row)
    }

    /// Re-push availability for the order's items after a reservation changes (master brands).
    async fn push_so_items(&self, row: &SoRow) -> SResult<Vec<String>> {
        if row.stock_mode != "master" {
            return Ok(Vec::new());
        }
        let items: Vec<(i64,)> = sqlx::query_as("SELECT item_id FROM so_lines WHERE so_id = ?")
            .bind(row.id)
            .fetch_all(&self.pool)
            .await?;
        let mut errors = Vec::new();
        for (item,) in items {
            errors.extend(
                self.push_stock(row.brand_id, Some(item), None)
                    .await?
                    .push_errors,
            );
        }
        Ok(errors)
    }

    async fn with_push_warnings(&self, id: i64, row: &SoRow) -> SResult<SalesOrder> {
        let errors = self.push_so_items(row).await?;
        let mut so = self.so(id).await?;
        so.warnings.extend(
            errors
                .into_iter()
                .map(|e| format!("channel not updated yet (the next sync retries): {e}")),
        );
        Ok(so)
    }

    /// Confirming reserves the stock; warehouse defaults to the brand's first one.
    pub async fn confirm_so(&self, who: &Principal, id: i64) -> SResult<SalesOrder> {
        who.require(Scope::Approve)?;
        human(who, "confirm sales orders")?;
        let row = self.so_row(id).await?;
        if row.warehouse_id.is_none() {
            sqlx::query(
                "UPDATE sales_orders SET warehouse_id = (SELECT id FROM warehouses WHERE brand_id = ?1 ORDER BY id LIMIT 1) WHERE id = ?2",
            )
            .bind(row.brand_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        }
        let row = self
            .so_transition(who, id, "proposed", "confirmed", "confirmed")
            .await?;
        self.with_push_warnings(id, &row).await
    }

    pub async fn reject_so(&self, who: &Principal, id: i64) -> SResult<SalesOrder> {
        who.require(Scope::Approve)?;
        human(who, "reject sales orders")?;
        self.so_transition(who, id, "proposed", "rejected", "rejected")
            .await?;
        self.so(id).await
    }

    /// Cancelling a confirmed order releases its reservation.
    pub async fn cancel_so(&self, who: &Principal, id: i64) -> SResult<SalesOrder> {
        who.require(Scope::Approve)?;
        human(who, "cancel sales orders")?;
        let row = self
            .so_transition(who, id, "confirmed", "cancelled", "cancelled")
            .await?;
        self.with_push_warnings(id, &row).await
    }

    /// Shipping a confirmed order: master brands record `sale` ledger entries at the order's
    /// warehouse in the same transaction as the status change.
    pub async fn fulfil_so(&self, who: &Principal, id: i64) -> SResult<SalesOrder> {
        who.require(Scope::Approve)?;
        human(who, "fulfil sales orders")?;
        let row = self.so_row(id).await?;
        let master = row.stock_mode == "master";
        let wh = match (master, row.warehouse_id) {
            (true, None) => return invalid(format!("SO #{id} has no warehouse to ship from")),
            (_, w) => w,
        };
        let mut tx = self.pool.begin().await?;
        let r = sqlx::query(
            "UPDATE sales_orders SET status = 'fulfilled', updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
             WHERE id = ? AND status = 'confirmed'",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() == 0 {
            return Err(ServiceError::Conflict(format!(
                "SO #{id} is {}, not confirmed",
                row.status
            )));
        }
        if let (true, Some(wh)) = (master, wh) {
            let lines: Vec<(i64, i64)> =
                sqlx::query_as("SELECT item_id, quantity FROM so_lines WHERE so_id = ?")
                    .bind(id)
                    .fetch_all(&mut *tx)
                    .await?;
            for (item, qty) in lines {
                crate::ledger::record_sale(&mut tx, item, wh, qty, id, &who.name).await?;
            }
        }
        tx.commit().await?;
        self.log(
            who,
            "fulfil_so",
            Some(("sales_order", id)),
            format!(
                "fulfilled SO #{id} for {}{}",
                row.customer,
                if master {
                    ""
                } else {
                    " (mirror brand: update store stock if the goods came from a store's stock)"
                }
            ),
        )
        .await?;
        self.with_push_warnings(id, &row).await
    }
}
