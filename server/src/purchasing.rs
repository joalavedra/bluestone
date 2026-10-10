//! Purchase orders: drafted by the agent or from the reorder list, approved and sent by a human,
//! received in full or in part. In master mode receipts land in the stock ledger and are pushed
//! to every channel; mirror-mode brands only track what arrived.

use crate::auth::{Principal, Scope};
use crate::service::{ItemFilter, SResult, Service, ServiceError, invalid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

const OPEN: &str = "('draft','approved','sent','partial')";

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct PoLine {
    pub id: i64,
    pub item_id: i64,
    pub sku: String,
    pub name: String,
    pub quantity: i64,
    pub received: i64,
    pub unit_cost: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct PurchaseOrder {
    pub id: i64,
    pub brand: String,
    pub supplier: Option<String>,
    pub supplier_email: Option<String>,
    pub warehouse: Option<String>,
    /// `draft` → `approved` → `sent` → `partial` → `received`, or `cancelled`.
    pub status: String,
    pub note: Option<String>,
    pub created_by: String,
    pub approved_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub units: i64,
    pub received_units: i64,
    pub total_cost: f64,
    pub lines: Vec<PoLine>,
}

#[derive(sqlx::FromRow)]
struct PoRow {
    id: i64,
    brand: String,
    supplier: Option<String>,
    supplier_email: Option<String>,
    warehouse: Option<String>,
    status: String,
    note: Option<String>,
    created_by: String,
    approved_by: Option<String>,
    created_at: String,
    updated_at: String,
}

const PO_SELECT: &str = "SELECT po.id, b.name AS brand, s.name AS supplier, s.email AS supplier_email, w.name AS warehouse,
  po.status, po.note, po.created_by, po.approved_by, po.created_at, po.updated_at
FROM purchase_orders po JOIN brands b ON b.id = po.brand_id
LEFT JOIN suppliers s ON s.id = po.supplier_id LEFT JOIN warehouses w ON w.id = po.warehouse_id";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftLine {
    /// Item id or SKU.
    pub item: String,
    pub quantity: i64,
    /// Defaults to the item's unit cost.
    pub unit_cost: Option<f64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftPo {
    /// Brand name or id.
    pub brand: String,
    /// Supplier name. Without `lines`, every low/out item of this supplier is added at its suggested quantity.
    pub supplier: Option<String>,
    pub lines: Option<Vec<DraftLine>>,
    /// Warehouse to receive into (master-mode brands). Defaults to the brand's first warehouse.
    pub warehouse: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReceiveLine {
    /// Item id or SKU.
    pub item: String,
    pub quantity: i64,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ReceivePo {
    /// Lines that arrived; omit to receive everything still outstanding.
    pub lines: Option<Vec<ReceiveLine>>,
}

#[derive(Debug, Serialize)]
pub struct Receipt {
    pub received_units: i64,
    /// Set for mirror-mode brands, whose stock lives in the stores and was not changed.
    pub note: Option<String>,
    pub push_errors: Vec<String>,
    pub po: PurchaseOrder,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PoListArgs {
    /// `open` (default), `all`, or one status.
    pub status: Option<String>,
    pub brand: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PoIdArgs {
    pub id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReceivePoArgs {
    pub id: i64,
    /// Lines that arrived; omit to receive everything outstanding.
    pub lines: Option<Vec<ReceiveLine>>,
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
    async fn po_view(&self, r: PoRow) -> SResult<PurchaseOrder> {
        let lines: Vec<PoLine> = sqlx::query_as(
            "SELECT pl.id, pl.item_id, i.sku, i.name, pl.quantity, pl.received, pl.unit_cost
             FROM po_lines pl JOIN items i ON i.id = pl.item_id WHERE pl.po_id = ? ORDER BY i.sku",
        )
        .bind(r.id)
        .fetch_all(&self.pool)
        .await?;
        Ok(PurchaseOrder {
            id: r.id,
            brand: r.brand,
            supplier: r.supplier,
            supplier_email: r.supplier_email,
            warehouse: r.warehouse,
            status: r.status,
            note: r.note,
            created_by: r.created_by,
            approved_by: r.approved_by,
            created_at: r.created_at,
            updated_at: r.updated_at,
            units: lines.iter().map(|l| l.quantity).sum(),
            received_units: lines.iter().map(|l| l.received).sum(),
            total_cost: lines
                .iter()
                .map(|l| l.quantity as f64 * l.unit_cost.unwrap_or(0.0))
                .sum(),
            lines,
        })
    }

    async fn po(&self, id: i64) -> SResult<PurchaseOrder> {
        let row: Option<PoRow> = sqlx::query_as(&format!("{PO_SELECT} WHERE po.id = ?"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => self.po_view(r).await,
            None => Err(ServiceError::NotFound(format!(
                "purchase order #{id} not found"
            ))),
        }
    }

    /// `status` may be a PO status or `open` (anything not received or cancelled).
    pub async fn purchase_orders(
        &self,
        who: &Principal,
        status: Option<&str>,
        brand: Option<&str>,
    ) -> SResult<Vec<PurchaseOrder>> {
        who.require(Scope::Read)?;
        let rows: Vec<PoRow> = sqlx::query_as(&format!(
            "{PO_SELECT} WHERE (?1 IS NULL OR po.status = ?1 OR (?1 = 'open' AND po.status IN {OPEN}))
               AND (?2 IS NULL OR b.name = ?2 COLLATE NOCASE) ORDER BY po.id DESC"
        ))
        .bind(status.filter(|s| !s.is_empty() && *s != "all"))
        .bind(brand)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            out.push(self.po_view(r).await?);
        }
        Ok(out)
    }

    pub async fn purchase_order(&self, who: &Principal, id: i64) -> SResult<PurchaseOrder> {
        who.require(Scope::Read)?;
        self.po(id).await
    }

    pub async fn draft_po(&self, who: &Principal, d: &DraftPo) -> SResult<PurchaseOrder> {
        who.require(Scope::Propose)?;
        let brand: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, name FROM brands WHERE CAST(id AS TEXT) = ?1 OR name = ?1 COLLATE NOCASE",
        )
        .bind(d.brand.trim())
        .fetch_optional(&self.pool)
        .await?;
        let Some((brand_id, brand)) = brand else {
            return Err(ServiceError::NotFound(format!(
                "brand `{}` not found",
                d.brand
            )));
        };
        let supplier: Option<(i64, String)> = match d.supplier.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(name) => Some(
                sqlx::query_as("SELECT id, name FROM suppliers WHERE name = ? COLLATE NOCASE")
                    .bind(name)
                    .fetch_optional(&self.pool)
                    .await?
                    .ok_or_else(|| {
                        ServiceError::NotFound(format!("supplier `{name}` not found"))
                    })?,
            ),
        };
        let mut lines: Vec<(i64, i64, Option<f64>)> = Vec::new();
        match d.lines.as_deref() {
            Some(given) if !given.is_empty() => {
                for l in given {
                    if l.quantity <= 0 {
                        return invalid(format!("{}: quantity must be positive", l.item));
                    }
                    let id = self.resolve_item(&l.item, Some(&brand)).await?;
                    let (item_brand, cost): (i64, Option<f64>) =
                        sqlx::query_as("SELECT brand_id, unit_cost FROM items WHERE id = ?")
                            .bind(id)
                            .fetch_one(&self.pool)
                            .await?;
                    if item_brand != brand_id {
                        return invalid(format!("{} is not a {brand} item", l.item));
                    }
                    if l.unit_cost.is_some_and(|c| !c.is_finite() || c < 0.0) {
                        return invalid(format!("{}: unit_cost must be ≥ 0", l.item));
                    }
                    lines.push((id, l.quantity, l.unit_cost.or(cost)));
                }
            }
            _ => {
                let Some((_, name)) = &supplier else {
                    return invalid("pass `lines`, or a `supplier` to draft from the reorder list");
                };
                let f = ItemFilter {
                    brand: Some(brand.clone()),
                    supplier: Some(name.clone()),
                    status: Some("attention".into()),
                    limit: Some(1000),
                    ..Default::default()
                };
                for i in self.search_items(&f).await? {
                    if i.suggested_reorder_qty > 0 {
                        lines.push((i.id, i.suggested_reorder_qty, i.unit_cost));
                    }
                }
                if lines.is_empty() {
                    return invalid(format!("nothing to reorder from {name} for {brand}"));
                }
            }
        }
        let warehouse: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM warehouses WHERE brand_id = ?1 AND (?2 IS NULL OR name = ?2 COLLATE NOCASE) ORDER BY id LIMIT 1",
        )
        .bind(brand_id)
        .bind(d.warehouse.as_deref())
        .fetch_optional(&self.pool)
        .await?;
        if let (Some(w), None) = (d.warehouse.as_deref(), warehouse) {
            return Err(ServiceError::NotFound(format!("warehouse `{w}` not found")));
        }
        let mut tx = self.pool.begin().await?;
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO purchase_orders (brand_id, supplier_id, warehouse_id, note, created_by) VALUES (?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(brand_id)
        .bind(supplier.as_ref().map(|s| s.0))
        .bind(warehouse.map(|w| w.0))
        .bind(d.note.as_deref())
        .bind(&who.name)
        .fetch_one(&mut *tx)
        .await?;
        for (item, qty, cost) in &lines {
            sqlx::query(
                "INSERT INTO po_lines (po_id, item_id, quantity, unit_cost) VALUES (?, ?, ?, ?)
                 ON CONFLICT (po_id, item_id) DO UPDATE SET quantity = quantity + excluded.quantity",
            )
            .bind(id)
            .bind(item)
            .bind(qty)
            .bind(cost)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        let units: i64 = lines.iter().map(|l| l.1).sum();
        let from = supplier
            .map(|s| format!(" from {}", s.1))
            .unwrap_or_default();
        self.log(
            who,
            "draft_po",
            Some(("purchase_order", id)),
            format!(
                "drafted PO #{id}{from}: {} lines, {units} units",
                lines.len()
            ),
        )
        .await?;
        self.po(id).await
    }

    async fn transition(
        &self,
        who: &Principal,
        id: i64,
        from: &[&str],
        to: &str,
        (verb, done): (&str, &str),
    ) -> SResult<PurchaseOrder> {
        let po = self.po(id).await?;
        let allowed = from
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(",");
        let r = sqlx::query(&format!(
            "UPDATE purchase_orders SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'),
               approved_by = CASE WHEN ?1 = 'approved' THEN ?2 ELSE approved_by END
             WHERE id = ?3 AND status IN ({allowed})"
        ))
        .bind(to)
        .bind(&who.name)
        .bind(id)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() == 0 {
            return Err(ServiceError::Conflict(format!(
                "PO #{id} is {}; cannot {verb} it",
                po.status
            )));
        }
        self.log(
            who,
            &format!("{verb}_po"),
            Some(("purchase_order", id)),
            format!("{done} PO #{id} ({} units)", po.units),
        )
        .await?;
        self.po(id).await
    }

    pub async fn approve_po(&self, who: &Principal, id: i64) -> SResult<PurchaseOrder> {
        who.require(Scope::Approve)?;
        human(who, "approve purchase orders")?;
        self.transition(who, id, &["draft"], "approved", ("approve", "approved"))
            .await
    }

    pub async fn send_po(&self, who: &Principal, id: i64) -> SResult<PurchaseOrder> {
        who.require(Scope::Approve)?;
        human(who, "send purchase orders")?;
        self.transition(who, id, &["approved"], "sent", ("send", "sent"))
            .await
    }

    /// Cancels an open PO; a partly received PO is closed short.
    pub async fn cancel_po(&self, who: &Principal, id: i64) -> SResult<PurchaseOrder> {
        who.require(Scope::Approve)?;
        human(who, "cancel purchase orders")?;
        self.transition(
            who,
            id,
            &["draft", "approved", "sent", "partial"],
            "cancelled",
            ("cancel", "cancelled"),
        )
        .await
    }

    pub async fn receive_po(
        &self,
        who: &Principal,
        id: i64,
        lines: Option<&[ReceiveLine]>,
    ) -> SResult<Receipt> {
        who.require(Scope::Approve)?;
        human(who, "receive goods")?;
        let po = self.po(id).await?;
        if !["approved", "sent", "partial"].contains(&po.status.as_str()) {
            return Err(ServiceError::Conflict(format!(
                "PO #{id} is {}; only approved or sent POs can be received",
                po.status
            )));
        }
        let (brand_id, mode, warehouse): (i64, String, Option<i64>) = sqlx::query_as(
            "SELECT po.brand_id, b.stock_mode, po.warehouse_id FROM purchase_orders po JOIN brands b ON b.id = po.brand_id WHERE po.id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        let mut receipts: Vec<(i64, i64, i64)> = Vec::new();
        match lines {
            Some(given) if !given.is_empty() => {
                for l in given {
                    let item = self.resolve_item(&l.item, Some(&po.brand)).await?;
                    let Some(line) = po.lines.iter().find(|x| x.item_id == item) else {
                        return invalid(format!("{} is not on PO #{id}", l.item));
                    };
                    let outstanding = line.quantity - line.received;
                    if l.quantity <= 0 || l.quantity > outstanding {
                        return invalid(format!("{}: can receive 1–{outstanding} units", line.sku));
                    }
                    receipts.push((line.id, item, l.quantity));
                }
            }
            _ => receipts.extend(
                po.lines
                    .iter()
                    .filter(|l| l.quantity > l.received)
                    .map(|l| (l.id, l.item_id, l.quantity - l.received)),
            ),
        }
        if receipts.is_empty() {
            return invalid(format!("nothing outstanding on PO #{id}"));
        }
        let master = mode == "master";
        let wh = match (master, warehouse) {
            // Drafted before the brand had warehouses: receive into its first one and remember it.
            (true, None) => {
                let first: Option<(i64,)> = sqlx::query_as(
                    "SELECT id FROM warehouses WHERE brand_id = ? ORDER BY id LIMIT 1",
                )
                .bind(brand_id)
                .fetch_optional(&self.pool)
                .await?;
                let Some((w,)) = first else {
                    return invalid(format!("PO #{id} has no warehouse to receive into"));
                };
                sqlx::query("UPDATE purchase_orders SET warehouse_id = ? WHERE id = ?")
                    .bind(w)
                    .bind(id)
                    .execute(&self.pool)
                    .await?;
                Some(w)
            }
            (_, w) => w,
        };
        let mut tx = self.pool.begin().await?;
        for (line, item, qty) in &receipts {
            // Guarded so repeated lines or concurrent receipts can never exceed the ordered quantity.
            let r = sqlx::query(
                "UPDATE po_lines SET received = received + ?1 WHERE id = ?2 AND received + ?1 <= quantity",
            )
            .bind(qty)
            .bind(line)
            .execute(&mut *tx)
            .await?;
            if r.rows_affected() == 0 {
                return Err(ServiceError::Conflict(format!(
                    "receiving {qty} more of item {item} would exceed what PO #{id} ordered; reload and retry"
                )));
            }
            if let (true, Some(wh)) = (master, wh) {
                crate::ledger::record_receipt(&mut tx, *item, wh, *qty, id, &who.name).await?;
            }
        }
        sqlx::query(
            "UPDATE purchase_orders SET updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'),
               status = CASE WHEN EXISTS (SELECT 1 FROM po_lines WHERE po_id = ?1 AND received < quantity) THEN 'partial' ELSE 'received' END
             WHERE id = ?1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let mut push_errors = Vec::new();
        if master {
            let mut items: Vec<i64> = receipts.iter().map(|r| r.1).collect();
            items.dedup();
            for item in items {
                push_errors.extend(
                    self.push_stock(brand_id, Some(item), None)
                        .await?
                        .push_errors,
                );
            }
        }
        let units: i64 = receipts.iter().map(|r| r.2).sum();
        self.log(
            who,
            "receive_po",
            Some(("purchase_order", id)),
            format!("received {units} units on PO #{id}"),
        )
        .await?;
        Ok(Receipt {
            received_units: units,
            note: (!master).then(|| {
                "brand mirrors its stores, so stock was not changed; update the stores or switch the brand to master mode".into()
            }),
            push_errors,
            po: self.po(id).await?,
        })
    }
}
