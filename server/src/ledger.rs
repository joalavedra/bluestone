//! Stock ledger: per-brand opt-in mode where Bluestone, not the stores, owns stock.
//!
//! In `master` mode every stock change is an append-only ledger entry (initial, sale, adjustment,
//! transfer, receipt, correction). Channel quantities are pushed from the ledger after each change
//! and on every sync; a channel showing anything else is drift and gets overwritten.
use crate::auth::{Principal, Scope};
use crate::connectors::{self, Connector, ListingRef};
use crate::service::*;
use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{FromRow, SqliteConnection};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug, Serialize, FromRow)]
pub struct Warehouse {
    pub id: i64,
    pub brand: String,
    pub name: String,
    /// Channel locations that stock from this warehouse (`channel/location`).
    pub locations: Option<String>,
    pub units: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct StockLevel {
    pub warehouse_id: i64,
    pub warehouse: String,
    pub quantity: i64,
}

#[derive(Debug, Serialize, FromRow)]
pub struct LedgerEntry {
    pub id: i64,
    pub warehouse: String,
    pub delta: i64,
    pub reason: String,
    pub ref_type: Option<String>,
    pub ref_id: Option<i64>,
    pub actor: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Reconcile {
    /// Units sold on the synced channel since the last sync, now deducted from the ledger.
    pub units_sold: i64,
    /// Channel stock rows updated to match the ledger.
    pub pushed: i64,
    /// Rows on the synced channel that disagreed with the ledger (overwritten).
    pub drift: i64,
    /// Item/warehouse pairs below zero.
    pub oversold: i64,
    pub push_errors: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProposeTransfer {
    /// Item id or SKU.
    pub item: String,
    pub brand: Option<String>,
    /// Source warehouse name.
    pub from: String,
    /// Destination warehouse name.
    pub to: String,
    pub quantity: i64,
    pub rationale: Option<String>,
}

async fn level(conn: &mut SqliteConnection, item: i64, wh: i64) -> SResult<i64> {
    let (q,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(delta), 0) FROM stock_ledger WHERE item_id = ? AND warehouse_id = ?",
    )
    .bind(item)
    .bind(wh)
    .fetch_one(&mut *conn)
    .await?;
    Ok(q)
}

#[allow(clippy::too_many_arguments)]
async fn entry(
    conn: &mut SqliteConnection,
    item: i64,
    wh: i64,
    delta: i64,
    reason: &str,
    reference: Option<(&str, i64)>,
    actor: &str,
    note: Option<&str>,
) -> SResult<()> {
    sqlx::query(
        "INSERT INTO stock_ledger (item_id, warehouse_id, delta, reason, ref_type, ref_id, actor, note) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(item)
    .bind(wh)
    .bind(delta)
    .bind(reason)
    .bind(reference.map(|r| r.0))
    .bind(reference.map(|r| r.1))
    .bind(actor)
    .bind(note)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Map unmapped channel locations of a brand to a warehouse of the same name (created on demand).
async fn map_locations(conn: &mut SqliteConnection, brand_id: i64) -> SResult<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO warehouses (brand_id, name)
         SELECT DISTINCT ?1, lo.name FROM locations lo JOIN channels c ON c.id = lo.channel_id
         WHERE c.brand_id = ?1 AND lo.warehouse_id IS NULL",
    )
    .bind(brand_id)
    .execute(&mut *conn)
    .await?;
    sqlx::query(
        "UPDATE locations SET warehouse_id = (SELECT w.id FROM warehouses w WHERE w.brand_id = ?1 AND w.name = locations.name)
         WHERE warehouse_id IS NULL AND channel_id IN (SELECT id FROM channels WHERE brand_id = ?1)",
    )
    .bind(brand_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Baseline item × warehouse pairs with no ledger history from what the channels report. When several channels
/// report the same item at the same warehouse they mirror one physical stock, so take the max.
async fn baseline(
    conn: &mut SqliteConnection,
    brand_id: i64,
    channel: Option<i64>,
    actor: &str,
) -> SResult<Vec<(i64, i64)>> {
    let rows: Vec<(i64, i64, i64)> = sqlx::query_as(
        "SELECT item_id, warehouse_id, MAX(qty) FROM (
           SELECT l.item_id, lo.warehouse_id, l.channel_id, SUM(ls.quantity) AS qty
           FROM listing_stock ls JOIN listings l ON l.id = ls.listing_id
           JOIN locations lo ON lo.id = ls.location_id JOIN channels c ON c.id = l.channel_id
           WHERE c.brand_id = ?1 AND (?2 IS NULL OR c.id = ?2) AND l.item_id IS NOT NULL AND lo.warehouse_id IS NOT NULL
             AND NOT EXISTS (SELECT 1 FROM stock_ledger s WHERE s.item_id = l.item_id AND s.warehouse_id = lo.warehouse_id)
           GROUP BY l.item_id, lo.warehouse_id, l.channel_id)
         GROUP BY item_id, warehouse_id",
    )
    .bind(brand_id)
    .bind(channel)
    .fetch_all(&mut *conn)
    .await?;
    let mut pairs = Vec::new();
    for (item, wh, qty) in rows {
        entry(
            conn,
            item,
            wh,
            qty,
            "initial",
            None,
            actor,
            Some("baseline from channel stock"),
        )
        .await?;
        pairs.push((item, wh));
    }
    Ok(pairs)
}

#[derive(FromRow)]
struct PushRow {
    listing_id: i64,
    location_id: i64,
    stock_ref: Option<String>,
    quantity: i64,
    item_id: i64,
    warehouse_id: i64,
    location: String,
    location_ext: String,
    channel_id: i64,
    channel: String,
    kind: String,
    base_url: String,
    credential_env: String,
    external_product_id: String,
    external_variant_id: String,
    inventory_ref: Option<String>,
    sku: String,
    expected: i64,
}

impl Service {
    async fn item_brand(&self, item_id: i64) -> SResult<(i64, String)> {
        sqlx::query_as(
            "SELECT b.id, b.stock_mode FROM items i JOIN brands b ON b.id = i.brand_id WHERE i.id = ?",
        )
        .bind(item_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| ServiceError::NotFound(format!("item {item_id} not found")))
    }

    pub async fn item_stock_mode(&self, item_id: i64) -> SResult<String> {
        Ok(self.item_brand(item_id).await?.1)
    }

    pub async fn warehouses(&self, brand: Option<&str>) -> SResult<Vec<Warehouse>> {
        Ok(sqlx::query_as(
            "SELECT w.id, b.name AS brand, w.name,
               (SELECT GROUP_CONCAT(c.name || '/' || lo.name, ', ') FROM locations lo JOIN channels c ON c.id = lo.channel_id WHERE lo.warehouse_id = w.id) AS locations,
               COALESCE((SELECT SUM(s.delta) FROM stock_ledger s WHERE s.warehouse_id = w.id), 0) AS units
             FROM warehouses w JOIN brands b ON b.id = w.brand_id
             WHERE (?1 IS NULL OR b.name = ?1 COLLATE NOCASE) ORDER BY b.name, w.name",
        )
        .bind(brand)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn stock_levels(&self, item_id: i64) -> SResult<Vec<StockLevel>> {
        Ok(sqlx::query_as(
            "SELECT w.id AS warehouse_id, w.name AS warehouse, COALESCE(SUM(s.delta), 0) AS quantity
             FROM items i JOIN warehouses w ON w.brand_id = i.brand_id
             LEFT JOIN stock_ledger s ON s.warehouse_id = w.id AND s.item_id = i.id
             WHERE i.id = ? GROUP BY w.id ORDER BY w.name",
        )
        .bind(item_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn ledger(&self, item_id: i64, limit: i64) -> SResult<Vec<LedgerEntry>> {
        Ok(sqlx::query_as(
            "SELECT s.id, w.name AS warehouse, s.delta, s.reason, s.ref_type, s.ref_id, s.actor, s.note, s.created_at
             FROM stock_ledger s JOIN warehouses w ON w.id = s.warehouse_id
             WHERE s.item_id = ? ORDER BY s.id DESC LIMIT ?",
        )
        .bind(item_id)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?)
    }

    /// Switch a brand between `mirror` (stores own stock) and `master` (the ledger owns stock).
    /// Entering master baselines the ledger from current channel stock and pushes it out.
    pub async fn set_stock_mode(
        &self,
        who: &Principal,
        brand: &str,
        mode: &str,
    ) -> SResult<Option<Reconcile>> {
        // Sync first so the master baseline includes sales made since the last sync.
        if mode == "master" {
            who.require(Scope::Admin)?;
            if who.kind != "human" && who.kind != "system" {
                return Err(ServiceError::Forbidden(
                    "only human tokens can change stock mode".into(),
                ));
            }
            let channels: Vec<(i64, String)> = sqlx::query_as(
                "SELECT c.id, c.name FROM channels c JOIN brands b ON b.id = c.brand_id
                 WHERE CAST(b.id AS TEXT) = ?1 OR b.name = ?1 COLLATE NOCASE",
            )
            .bind(brand.trim())
            .fetch_all(&self.pool)
            .await?;
            for (id, name) in channels {
                if let Err(e) = self.sync_channel(who, id).await {
                    return Err(ServiceError::Conflict(format!(
                        "sync of {name} failed, so the stock baseline would be stale; fix it and retry: {e}"
                    )));
                }
            }
        }
        self.switch_stock_mode(who, brand, mode).await
    }

    pub(crate) async fn switch_stock_mode(
        &self,
        who: &Principal,
        brand: &str,
        mode: &str,
    ) -> SResult<Option<Reconcile>> {
        who.require(Scope::Admin)?;
        if who.kind != "human" && who.kind != "system" {
            return Err(ServiceError::Forbidden(
                "only human tokens can change stock mode".into(),
            ));
        }
        if mode != "mirror" && mode != "master" {
            return invalid("mode must be mirror or master");
        }
        let (brand_id, name, current): (i64, String, String) = sqlx::query_as(
            "SELECT id, name, stock_mode FROM brands WHERE CAST(id AS TEXT) = ?1 OR name = ?1 COLLATE NOCASE",
        )
        .bind(brand.trim())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| ServiceError::NotFound(format!("brand `{brand}` not found")))?;
        if current == mode {
            return invalid(format!("{name} is already in {mode} mode"));
        }
        let mut tx = self.pool.begin().await?;
        let mut seeded = 0;
        if mode == "master" {
            map_locations(&mut tx, brand_id).await?;
            // Re-entering master: the stores may have moved while mirrored, so re-baseline from them.
            let drift: Vec<(i64, i64, i64)> = sqlx::query_as(
                "SELECT item_id, warehouse_id, MAX(qty) FROM (
                   SELECT l.item_id, lo.warehouse_id, l.channel_id, SUM(ls.quantity) AS qty
                   FROM listing_stock ls JOIN listings l ON l.id = ls.listing_id
                   JOIN locations lo ON lo.id = ls.location_id JOIN channels c ON c.id = l.channel_id
                   WHERE c.brand_id = ? AND l.item_id IS NOT NULL AND lo.warehouse_id IS NOT NULL
                     AND EXISTS (SELECT 1 FROM stock_ledger s WHERE s.item_id = l.item_id)
                   GROUP BY l.item_id, lo.warehouse_id, l.channel_id)
                 GROUP BY item_id, warehouse_id",
            )
            .bind(brand_id)
            .fetch_all(&mut *tx)
            .await?;
            for (item, wh, qty) in drift {
                let have = level(&mut tx, item, wh).await?;
                if have != qty {
                    entry(
                        &mut tx,
                        item,
                        wh,
                        qty - have,
                        "correction",
                        None,
                        &who.name,
                        Some("re-baseline from channels on entering master mode"),
                    )
                    .await?;
                    seeded += 1;
                }
            }
            seeded += baseline(&mut tx, brand_id, None, &who.name).await?.len() as i64;
            sqlx::query("UPDATE order_lines SET ledgered = 1 WHERE ledgered = 0 AND channel_id IN (SELECT id FROM channels WHERE brand_id = ?)")
                .bind(brand_id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE brands SET stock_mode = ?, master_since = CASE WHEN ? = 'master' THEN ? ELSE master_since END WHERE id = ?")
            .bind(mode)
            .bind(mode)
            .bind(now())
            .bind(brand_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.log(
            who,
            "stock_mode",
            Some(("brand", brand_id)),
            if mode == "master" {
                format!("{name} → master mode: Bluestone owns stock ({seeded} ledger baselines)")
            } else {
                format!("{name} → mirror mode: stores own stock again")
            },
        )
        .await?;
        if mode == "master" {
            Ok(Some(self.push_stock(brand_id, None, None).await?))
        } else {
            Ok(None)
        }
    }

    /// After a successful channel sync of a master brand: ledger new sales, then push the ledger out.
    pub(crate) async fn reconcile(&self, channel_id: i64) -> SResult<Option<Reconcile>> {
        let (brand_id, mode, since): (i64, String, Option<String>) = sqlx::query_as(
            "SELECT b.id, b.stock_mode, b.master_since FROM channels c JOIN brands b ON b.id = c.brand_id WHERE c.id = ?",
        )
        .bind(channel_id)
        .fetch_one(&self.pool)
        .await?;
        if mode != "master" {
            return Ok(None);
        }
        let mut tx = self.pool.begin().await?;
        map_locations(&mut tx, brand_id).await?;
        // A fresh baseline already reflects this channel's unledgered sales at its sales warehouse.
        for (item, wh) in baseline(&mut tx, brand_id, Some(channel_id), "system").await? {
            sqlx::query(
                "UPDATE order_lines SET ledgered = 1 WHERE channel_id = ?1 AND ledgered = 0
                   AND listing_id IN (SELECT id FROM listings WHERE item_id = ?2)
                   AND ?3 = (SELECT warehouse_id FROM locations WHERE channel_id = ?1 AND warehouse_id IS NOT NULL ORDER BY id LIMIT 1)",
            )
            .bind(channel_id)
            .bind(item)
            .bind(wh)
            .execute(&mut *tx)
            .await?;
        }
        // Orders placed before master mode began are history, not stock movements.
        sqlx::query("UPDATE order_lines SET ledgered = 1 WHERE channel_id = ? AND ledgered = 0 AND ordered_at < ?")
            .bind(channel_id)
            .bind(since.unwrap_or_default())
            .execute(&mut *tx)
            .await?;
        let default_wh: Option<(i64,)> = sqlx::query_as(
            "SELECT warehouse_id FROM locations WHERE channel_id = ? AND warehouse_id IS NOT NULL ORDER BY id LIMIT 1",
        )
        .bind(channel_id)
        .fetch_optional(&mut *tx)
        .await?;
        let mut units_sold = 0;
        if let Some((wh,)) = default_wh {
            let sales: Vec<(i64, i64, i64, String)> = sqlx::query_as(
                "SELECT o.id, l.item_id, o.quantity, o.external_order_id FROM order_lines o JOIN listings l ON l.id = o.listing_id
                 WHERE o.channel_id = ? AND o.ledgered = 0 AND l.item_id IS NOT NULL ORDER BY o.id",
            )
            .bind(channel_id)
            .fetch_all(&mut *tx)
            .await?;
            for (line, item, qty, order) in sales {
                let note = format!("order {order}");
                entry(
                    &mut tx,
                    item,
                    wh,
                    -qty,
                    "sale",
                    Some(("order_line", line)),
                    "system",
                    Some(&note),
                )
                .await?;
                sqlx::query("UPDATE order_lines SET ledgered = 1 WHERE id = ?")
                    .bind(line)
                    .execute(&mut *tx)
                    .await?;
                units_sold += qty;
            }
        }
        tx.commit().await?;
        let mut r = self.push_stock(brand_id, None, Some(channel_id)).await?;
        r.units_sold = units_sold;
        Ok(Some(r))
    }

    /// Make every channel show the ledger quantity for its warehouse. `synced` is the channel whose
    /// stock was just read: differences there are drift. Elsewhere they are expected propagation.
    pub(crate) async fn push_stock(
        &self,
        brand_id: i64,
        only_item: Option<i64>,
        synced: Option<i64>,
    ) -> SResult<Reconcile> {
        let rows: Vec<PushRow> = sqlx::query_as(
            "SELECT ls.listing_id, ls.location_id, ls.stock_ref, ls.quantity, l.item_id, lo.warehouse_id,
               lo.name AS location, lo.external_id AS location_ext, c.id AS channel_id, c.name AS channel, c.kind, c.base_url,
               c.credential_env, l.external_product_id, l.external_variant_id, l.inventory_ref, i.sku,
               COALESCE((SELECT SUM(s.delta) FROM stock_ledger s WHERE s.item_id = l.item_id AND s.warehouse_id = lo.warehouse_id), 0) AS expected
             FROM listing_stock ls JOIN listings l ON l.id = ls.listing_id JOIN locations lo ON lo.id = ls.location_id
             JOIN channels c ON c.id = l.channel_id JOIN items i ON i.id = l.item_id
             WHERE c.brand_id = ?1 AND lo.warehouse_id IS NOT NULL AND (?2 IS NULL OR l.item_id = ?2)
               AND COALESCE(l.status, '') <> 'removed'
             ORDER BY c.id, l.id",
        )
        .bind(brand_id)
        .bind(only_item)
        .fetch_all(&self.pool)
        .await?;
        let system = Principal::system();
        let mut r = Reconcile::default();
        let mut oversold = BTreeSet::new();
        let mut conns: HashMap<i64, Result<Box<dyn Connector>, String>> = HashMap::new();
        for row in rows {
            if row.expected < 0 && oversold.insert((row.item_id, row.warehouse_id)) {
                self.log(
                    &system,
                    "oversold",
                    Some(("item", row.item_id)),
                    format!(
                        "{} is oversold by {} at {}",
                        row.sku, -row.expected, row.location
                    ),
                )
                .await?;
            }
            let target = row.expected.max(0);
            if row.quantity == target {
                continue;
            }
            if synced == Some(row.channel_id) {
                r.drift += 1;
                self.log(
                    &system,
                    "drift",
                    Some(("item", row.item_id)),
                    format!(
                        "{}: {} showed {} at {}, ledger says {target} — overwriting",
                        row.sku, row.channel, row.quantity, row.location
                    ),
                )
                .await?;
            }
            let conn = conns.entry(row.channel_id).or_insert_with(|| {
                std::env::var(&row.credential_env)
                    .map_err(|_| format!("env var {} is not set", row.credential_env))
                    .and_then(|cred| {
                        connectors::build(&row.kind, &row.base_url, cred)
                            .map_err(|e| format!("{e:#}"))
                    })
            });
            let target_ref = ListingRef {
                external_product_id: row.external_product_id.clone(),
                external_variant_id: row.external_variant_id.clone(),
                inventory_ref: row.inventory_ref.clone(),
            };
            let res = match conn {
                Ok(c) => c
                    .set_stock(
                        &target_ref,
                        &row.location_ext,
                        row.stock_ref.as_deref(),
                        row.quantity,
                        target,
                    )
                    .await
                    .map_err(|e| format!("{e:#}")),
                Err(e) => Err(e.clone()),
            };
            match res {
                Ok(()) => {
                    sqlx::query("UPDATE listing_stock SET quantity = ?, updated_at = ? WHERE listing_id = ? AND location_id = ?")
                        .bind(target)
                        .bind(now())
                        .bind(row.listing_id)
                        .bind(row.location_id)
                        .execute(&self.pool)
                        .await?;
                    r.pushed += 1;
                }
                Err(e) => r.push_errors.push(format!(
                    "{} on {} at {}: {e}",
                    row.sku, row.channel, row.location
                )),
            }
        }
        r.oversold = oversold.len() as i64;
        if !r.push_errors.is_empty() {
            self.log(
                &system,
                "push_failed",
                None,
                format!(
                    "{} stock pushes failed: {}",
                    r.push_errors.len(),
                    r.push_errors.join("; ")
                ),
            )
            .await?;
        }
        Ok(r)
    }

    /// Master-mode stock proposal: the target is a warehouse and the change lands in the ledger.
    pub(crate) async fn propose_ledger_stock(
        &self,
        who: &Principal,
        item_id: i64,
        p: &ProposeStock,
    ) -> SResult<ProposalView> {
        let levels = self.stock_levels(item_id).await?;
        let matches: Vec<&StockLevel> = levels
            .iter()
            .filter(|l| {
                p.location
                    .as_deref()
                    .is_none_or(|n| l.warehouse.eq_ignore_ascii_case(n))
            })
            .collect();
        let lvl = match matches.as_slice() {
            [one] => *one,
            [] => {
                return Err(ServiceError::NotFound(
                    "no such warehouse for this brand".into(),
                ));
            }
            _ => {
                return invalid(format!(
                    "brand has several warehouses ({}); pass `location`",
                    levels
                        .iter()
                        .map(|l| l.warehouse.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        };
        let quantity = match (p.quantity, p.delta) {
            (Some(q), None) => q,
            (None, Some(d)) => lvl.quantity + d,
            _ => return invalid("pass exactly one of `quantity` or `delta`"),
        };
        if quantity < 0 {
            return invalid("stock cannot go below 0");
        }
        if quantity == lvl.quantity {
            return invalid(format!("stock is already {quantity}"));
        }
        let at = |q: i64| json!({ "warehouse_id": lvl.warehouse_id, "location": lvl.warehouse, "quantity": q });
        self.insert_proposal(
            who,
            "ledger_adjustment",
            item_id,
            None,
            at(lvl.quantity),
            at(quantity),
            p.rationale.as_deref(),
        )
        .await
    }

    pub async fn propose_transfer(
        &self,
        who: &Principal,
        t: &ProposeTransfer,
    ) -> SResult<ProposalView> {
        who.require(Scope::Propose)?;
        let item_id = self.resolve_item(&t.item, t.brand.as_deref()).await?;
        if self.item_stock_mode(item_id).await? != "master" {
            return invalid("transfers need the brand in master mode");
        }
        if t.quantity <= 0 {
            return invalid("quantity must be > 0");
        }
        let levels = self.stock_levels(item_id).await?;
        let find = |n: &str| {
            levels
                .iter()
                .find(|l| l.warehouse.eq_ignore_ascii_case(n))
                .ok_or_else(|| ServiceError::NotFound(format!("no warehouse named `{n}`")))
        };
        let (from, to) = (find(&t.from)?, find(&t.to)?);
        if from.warehouse_id == to.warehouse_id {
            return invalid("from and to must differ");
        }
        if from.quantity < t.quantity {
            return invalid(format!("{} only has {}", from.warehouse, from.quantity));
        }
        let side = |f: i64, to_q: i64| {
            json!({ "from_id": from.warehouse_id, "from": from.warehouse, "from_qty": f,
                    "to_id": to.warehouse_id, "to": to.warehouse, "to_qty": to_q, "quantity": t.quantity })
        };
        self.insert_proposal(
            who,
            "stock_transfer",
            item_id,
            None,
            side(from.quantity, to.quantity),
            side(from.quantity - t.quantity, to.quantity + t.quantity),
            t.rationale.as_deref(),
        )
        .await
    }

    /// Applies ledger proposals; returns false for channel proposals so the caller handles them.
    pub(crate) async fn apply_ledger(&self, p: &ProposalView) -> anyhow::Result<bool> {
        let item = match p.item_id {
            Some(i) => i,
            None => return Ok(false),
        };
        let (brand_id, mode) = self.item_brand(item).await?;
        let mut tx = self.pool.begin().await?;
        match p.kind.as_str() {
            "stock_adjustment" if mode == "master" => {
                anyhow::bail!("brand is now in master mode; re-propose against a warehouse")
            }
            "ledger_adjustment" | "stock_transfer" if mode != "master" => {
                anyhow::bail!("brand is back in mirror mode; re-propose against a channel")
            }
            "ledger_adjustment" => {
                let wh = p.after["warehouse_id"]
                    .as_i64()
                    .context("missing warehouse")?;
                let want = p.after["quantity"].as_i64().unwrap_or_default();
                let was = p.before["quantity"].as_i64().unwrap_or_default();
                let have = level(&mut tx, item, wh).await?;
                if have != was {
                    anyhow::bail!("stock is now {have}, not {was}; re-propose");
                }
                entry(
                    &mut tx,
                    item,
                    wh,
                    want - have,
                    "adjustment",
                    Some(("proposal", p.id)),
                    &p.actor,
                    p.rationale.as_deref(),
                )
                .await?;
            }
            "stock_transfer" => {
                let from = p.after["from_id"].as_i64().context("missing from")?;
                let to = p.after["to_id"].as_i64().context("missing to")?;
                let q = p.after["quantity"].as_i64().unwrap_or_default();
                let have = level(&mut tx, item, from).await?;
                if have < q {
                    anyhow::bail!("source warehouse only has {have}");
                }
                for (wh, d) in [(from, -q), (to, q)] {
                    entry(
                        &mut tx,
                        item,
                        wh,
                        d,
                        "transfer",
                        Some(("proposal", p.id)),
                        &p.actor,
                        p.rationale.as_deref(),
                    )
                    .await?;
                }
            }
            _ => return Ok(false),
        }
        tx.commit().await?;
        let r = self.push_stock(brand_id, Some(item), None).await?;
        if !r.push_errors.is_empty() {
            // The ledger is the truth and the next sync retries; surface the gap on the proposal.
            sqlx::query("UPDATE proposals SET error = ? WHERE id = ?")
                .bind(format!(
                    "applied to the ledger, but these channels were not updated yet: {}",
                    r.push_errors.join("; ")
                ))
                .bind(p.id)
                .execute(&self.pool)
                .await?;
        }
        Ok(true)
    }
}

/// Merging two items that are the same physical stock: keep the larger quantity per warehouse.
pub(crate) async fn merge_ledgers(
    conn: &mut SqliteConnection,
    source: i64,
    target: i64,
    actor: &str,
) -> SResult<()> {
    let whs: Vec<(i64,)> =
        sqlx::query_as("SELECT DISTINCT warehouse_id FROM stock_ledger WHERE item_id = ?")
            .bind(source)
            .fetch_all(&mut *conn)
            .await?;
    for (wh,) in whs {
        let src = level(conn, source, wh).await?;
        let tgt = level(conn, target, wh).await?;
        if src != 0 {
            entry(
                conn,
                source,
                wh,
                -src,
                "correction",
                Some(("item", target)),
                actor,
                Some("merged into another item"),
            )
            .await?;
        }
        if src > tgt {
            entry(
                conn,
                target,
                wh,
                src - tgt,
                "correction",
                Some(("item", source)),
                actor,
                Some("merge: kept the larger quantity"),
            )
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn svc() -> Service {
        let path = std::env::temp_dir().join(format!("bluestone-{}.db", uuid::Uuid::new_v4()));
        let pool = crate::db::connect(&format!("sqlite://{}", path.display()))
            .await
            .unwrap();
        for sql in [
            "INSERT INTO brands (id, name) VALUES (1, 'B')",
            "INSERT INTO channels (id, brand_id, kind, name, base_url, credential_env) VALUES (1, 1, 'shopify', 'shop', 'http://x', 'NOPE_UNSET'), (2, 1, 'prestashop', 'presta', 'http://y', 'NOPE_UNSET')",
            "INSERT INTO locations (id, channel_id, external_id, name) VALUES (1, 1, 'L1', 'Main'), (2, 2, '1', 'Main'), (3, 1, 'L2', 'Overflow')",
            "INSERT INTO items (id, brand_id, sku, name) VALUES (1, 1, 'A', 'Alpha')",
            "INSERT INTO listings (id, channel_id, item_id, external_product_id, title) VALUES (1, 1, 1, 'p1', 'Alpha'), (2, 2, 1, 'p2', 'Alpha')",
            "INSERT INTO listing_stock (listing_id, location_id, quantity) VALUES (1, 1, 10), (2, 2, 8), (1, 3, 4)",
            "INSERT INTO order_lines (channel_id, external_order_id, external_line_id, listing_id, quantity, ordered_at) VALUES (1, 'old', '1', 1, 3, '2020-01-01T00:00:00Z')",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        Service::new(pool)
    }

    fn levels(v: &[StockLevel]) -> Vec<(String, i64)> {
        v.iter()
            .map(|l| (l.warehouse.clone(), l.quantity))
            .collect()
    }

    #[tokio::test]
    async fn master_mode_baselines_ledgers_sales_and_flags_oversell() {
        let s = svc().await;
        let me = Principal::system();
        s.switch_stock_mode(&me, "B", "master").await.unwrap();
        // Shared "Main" warehouse takes the max of the two channels, not the sum; history is not deducted.
        assert_eq!(
            levels(&s.stock_levels(1).await.unwrap()),
            vec![("Main".into(), 10), ("Overflow".into(), 4)]
        );
        assert_eq!(s.get_item(1).await.unwrap().summary.on_hand, 14);

        sqlx::query("INSERT INTO order_lines (channel_id, external_order_id, external_line_id, listing_id, quantity, ordered_at) VALUES (1, 'new', '1', 1, 12, '2999-01-01T00:00:00Z')")
            .execute(&s.pool)
            .await
            .unwrap();
        let r = s.reconcile(1).await.unwrap().unwrap();
        assert_eq!(r.units_sold, 12);
        assert_eq!(r.oversold, 1);
        assert_eq!(s.stock_levels(1).await.unwrap()[0].quantity, -2);
        // Pushing fails without credentials, and is reported rather than silently dropped.
        assert!(!r.push_errors.is_empty());
        // A second reconcile does not deduct the same sale twice.
        assert_eq!(s.reconcile(1).await.unwrap().unwrap().units_sold, 0);
    }

    #[tokio::test]
    async fn merging_items_keeps_the_larger_quantity() {
        let s = svc().await;
        let me = Principal::system();
        for sql in [
            "INSERT INTO items (id, brand_id, sku, name) VALUES (2, 1, 'A2', 'Alpha dup')",
            "INSERT INTO listings (id, channel_id, item_id, external_product_id, title) VALUES (3, 2, 2, 'p3', 'Alpha dup')",
            "INSERT INTO listing_stock (listing_id, location_id, quantity) VALUES (3, 2, 12)",
        ] {
            sqlx::query(sql).execute(&s.pool).await.unwrap();
        }
        s.switch_stock_mode(&me, "B", "master").await.unwrap();
        s.merge_items(&me, 2, 1).await.unwrap();
        assert_eq!(
            levels(&s.stock_levels(1).await.unwrap()),
            vec![("Main".into(), 12), ("Overflow".into(), 4)]
        );
        assert!(
            s.stock_levels(2)
                .await
                .unwrap()
                .iter()
                .all(|l| l.quantity == 0)
        );
    }

    #[tokio::test]
    async fn activation_refuses_when_a_channel_cannot_sync() {
        let s = svc().await;
        assert!(
            s.set_stock_mode(&Principal::system(), "B", "master")
                .await
                .is_err()
        );
        assert_eq!(s.item_stock_mode(1).await.unwrap(), "mirror");
    }

    #[tokio::test]
    async fn transfers_and_adjustments_go_through_the_ledger() {
        let s = svc().await;
        let me = Principal::system();
        s.switch_stock_mode(&me, "B", "master").await.unwrap();
        let t = s
            .propose_transfer(
                &me,
                &ProposeTransfer {
                    item: "A".into(),
                    brand: None,
                    from: "main".into(),
                    to: "Overflow".into(),
                    quantity: 3,
                    rationale: None,
                },
            )
            .await
            .unwrap();
        assert!(s.apply_ledger(&t).await.unwrap());
        assert_eq!(
            levels(&s.stock_levels(1).await.unwrap()),
            vec![("Main".into(), 7), ("Overflow".into(), 7)]
        );
        // A stale adjustment (stock moved since it was proposed) is refused.
        let adj = s
            .propose_ledger_stock(
                &me,
                1,
                &serde_json::from_value(json!({ "item": "A", "location": "Main", "quantity": 20 }))
                    .unwrap(),
            )
            .await
            .unwrap();
        let t2 = s
            .propose_transfer(
                &me,
                &ProposeTransfer {
                    item: "A".into(),
                    brand: None,
                    from: "Main".into(),
                    to: "Overflow".into(),
                    quantity: 1,
                    rationale: None,
                },
            )
            .await
            .unwrap();
        s.apply_ledger(&t2).await.unwrap();
        assert!(s.apply_ledger(&adj).await.is_err());
        assert!(
            s.propose_transfer(
                &me,
                &ProposeTransfer {
                    item: "A".into(),
                    brand: None,
                    from: "Main".into(),
                    to: "Overflow".into(),
                    quantity: 99,
                    rationale: None,
                },
            )
            .await
            .is_err()
        );
    }
}
