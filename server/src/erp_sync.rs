//! The brand's ERP (Holded, Odoo, ICG, ...) next to its shops: products are pulled and matched to
//! items by SKU, invoices are read on demand, confirmed sales orders are pushed as ERP orders.
//! Connecting and pushing are human-only, like accounting; agents can read.

use crate::auth::{Principal, Scope};
use crate::erp::{self, Erp, Order, OrderLine};
use crate::service::{SResult, Service, ServiceError, invalid};
use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct NewErpConnection {
    pub brand: String,
    /// `holded`.
    pub kind: String,
    pub base_url: Option<String>,
    /// NAME, env:NAME, nango:<integration>/<connection> or secret:<id>.
    pub credential_env: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ErpConnection {
    pub id: i64,
    pub brand: String,
    pub kind: String,
    pub base_url: String,
    pub last_synced_at: Option<String>,
    pub last_sync_error: Option<String>,
    /// Products in the last pull, and how many share a SKU with a Bluestone item.
    pub products: i64,
    pub matched: i64,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ErpOrder {
    pub id: i64,
    pub connection_id: i64,
    pub brand: String,
    pub system: String,
    pub sales_order_id: i64,
    /// `pending`, `exported` or `failed`.
    pub status: String,
    pub external_id: Option<String>,
    pub error: Option<String>,
    pub created_by: Option<String>,
    pub updated_at: String,
}

/// An item's product in its brand's ERP.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ErpLink {
    pub system: String,
    pub code: String,
    pub name: String,
    pub stock: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ErpOverview {
    pub connections: Vec<ErpConnection>,
    pub orders: Vec<ErpOrder>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ErpInvoiceArgs {
    pub brand: String,
    /// First day, `YYYY-MM-DD`.
    pub from: String,
    /// Last day (inclusive), `YYYY-MM-DD`.
    pub to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ErpBrandArgs {
    pub brand: String,
}

#[derive(sqlx::FromRow)]
struct ConnRow {
    id: i64,
    kind: String,
    base_url: String,
    credential_env: String,
}

const CONN_SELECT: &str =
    "SELECT c.id, b.name AS brand, c.kind, c.base_url, c.last_synced_at, c.last_sync_error,
  (SELECT COUNT(*) FROM erp_products p WHERE p.connection_id = c.id) AS products,
  (SELECT COUNT(*) FROM erp_products p WHERE p.connection_id = c.id AND (
    EXISTS (SELECT 1 FROM items i WHERE i.brand_id = c.brand_id AND i.sku = p.sku COLLATE NOCASE)
    OR EXISTS (SELECT 1 FROM listings l JOIN items i ON i.id = l.item_id
               WHERE i.brand_id = c.brand_id AND l.sku = p.sku COLLATE NOCASE))) AS matched
FROM erp_connections c JOIN brands b ON b.id = c.brand_id WHERE c.active = 1";

const ORDER_SELECT: &str = "SELECT o.id, o.connection_id, b.name AS brand, c.kind AS system, o.sales_order_id, o.status,
  o.external_id, o.error, o.created_by, o.updated_at
FROM erp_orders o JOIN erp_connections c ON c.id = o.connection_id JOIN brands b ON b.id = c.brand_id";

fn human(who: &Principal) -> SResult<()> {
    who.require(Scope::Approve)?;
    if who.kind != "human" && who.kind != "system" {
        return Err(ServiceError::Forbidden(
            "only human tokens can write to the ERP".into(),
        ));
    }
    Ok(())
}

fn is_human(who: &Principal) -> bool {
    who.kind == "human" || who.kind == "system"
}

/// Provider errors can carry ERP details; agents get a generic message (the server log keeps it).
fn provider_error(who: &Principal, e: anyhow::Error, what: &str) -> ServiceError {
    if is_human(who) {
        ServiceError::Internal(e.context(format!("{what} failed")))
    } else {
        tracing::warn!(error = %format!("{e:#}"), "{what} failed for {}", who.name);
        ServiceError::Internal(anyhow::anyhow!(
            "{what} failed (details visible to human tokens)"
        ))
    }
}

impl Service {
    pub async fn set_erp(&self, who: &Principal, c: &NewErpConnection) -> SResult<i64> {
        who.require(Scope::Admin)?;
        human(who)?;
        let Some(default_url) = erp::default_base_url(&c.kind) else {
            return invalid(format!("ERP kind must be one of {}", erp::KINDS.join(", ")));
        };
        if c.credential_env == crate::credentials::OAUTH {
            return invalid(
                "ERP credentials must be an env var, nango:<integration>/<connection> or a stored key",
            );
        }
        let (brand_id,): (i64,) = sqlx::query_as(
            "INSERT INTO brands (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name = excluded.name RETURNING id",
        )
        .bind(&c.brand)
        .fetch_one(&self.pool)
        .await?;
        let base_url = c
            .base_url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| default_url.into());
        let mut tx = self.pool.begin().await?;
        // Reconnecting the same ERP reuses its connection, so earlier pushed orders still count.
        let same: Option<(i64,)> = sqlx::query_as(
            "SELECT id FROM erp_connections WHERE brand_id = ? AND kind = ? AND base_url = ? ORDER BY id DESC LIMIT 1",
        )
        .bind(brand_id)
        .bind(&c.kind)
        .bind(&base_url)
        .fetch_optional(&mut *tx)
        .await?;
        sqlx::query("UPDATE erp_connections SET active = 0 WHERE brand_id = ?")
            .bind(brand_id)
            .execute(&mut *tx)
            .await?;
        let id = if let Some((id,)) = same {
            sqlx::query("UPDATE erp_connections SET credential_env = ?, active = 1 WHERE id = ?")
                .bind(&c.credential_env)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            id
        } else {
            sqlx::query_as::<_, (i64,)>(
                "INSERT INTO erp_connections (brand_id, kind, base_url, credential_env) VALUES (?, ?, ?, ?) RETURNING id",
            )
            .bind(brand_id)
            .bind(&c.kind)
            .bind(&base_url)
            .bind(&c.credential_env)
            .fetch_one(&mut *tx)
            .await?
            .0
        };
        tx.commit().await?;
        self.log(
            who,
            "erp_connect",
            None,
            format!("{} now uses {} as its ERP ({base_url})", c.brand, c.kind),
        )
        .await?;
        Ok(id)
    }

    pub async fn erp(&self, who: &Principal) -> SResult<ErpOverview> {
        who.require(Scope::Read)?;
        let mut connections: Vec<ErpConnection> =
            sqlx::query_as(&format!("{CONN_SELECT} ORDER BY b.name"))
                .fetch_all(&self.pool)
                .await?;
        let mut orders: Vec<ErpOrder> = sqlx::query_as(&format!(
            "{ORDER_SELECT} ORDER BY o.updated_at DESC, o.id DESC LIMIT 500"
        ))
        .fetch_all(&self.pool)
        .await?;
        // Provider errors can carry ERP details; agents only see that something failed.
        if !is_human(who) {
            for c in &mut connections {
                if c.last_sync_error.is_some() {
                    c.last_sync_error =
                        Some("sync failed (details visible to human tokens)".into());
                }
            }
            for o in &mut orders {
                if o.error.is_some() {
                    o.error = Some("push failed (details visible to human tokens)".into());
                }
            }
        }
        Ok(ErpOverview {
            connections,
            orders,
        })
    }

    async fn erp_conn(&self, brand: &str) -> SResult<ConnRow> {
        sqlx::query_as(
            "SELECT c.id, c.kind, c.base_url, c.credential_env FROM erp_connections c JOIN brands b ON b.id = c.brand_id
             WHERE b.name = ? AND c.active = 1",
        )
        .bind(brand)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::Invalid(format!(
                "{brand} has no ERP; connect one in Settings → Integrations"
            ))
        })
    }

    async fn erp_client(&self, c: &ConnRow) -> anyhow::Result<Box<dyn Erp>> {
        let credential = crate::credentials::resolve(&self.pool, 0, &c.kind, &c.credential_env)
            .await
            .context("ERP credential")?;
        erp::build(&c.kind, &c.base_url, credential)
    }

    /// Pulls the ERP's products (code, SKU, stock), replacing the previous pull.
    pub async fn sync_erp(&self, who: &Principal, brand: &str) -> SResult<ErpConnection> {
        who.require(Scope::Read)?;
        let conn = self.erp_conn(brand).await?;
        let pulled = async { self.erp_client(&conn).await?.products().await }.await;
        let products = match pulled {
            Ok(p) => p,
            Err(e) => {
                sqlx::query("UPDATE erp_connections SET last_sync_error = ? WHERE id = ?")
                    .bind(format!("{e:#}"))
                    .bind(conn.id)
                    .execute(&self.pool)
                    .await?;
                return Err(provider_error(
                    who,
                    e,
                    &format!("{} sync", erp::label(&conn.kind)),
                ));
            }
        };
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM erp_products WHERE connection_id = ?")
            .bind(conn.id)
            .execute(&mut *tx)
            .await?;
        for p in &products {
            sqlx::query(
                "INSERT OR REPLACE INTO erp_products (connection_id, code, sku, name, stock) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(conn.id)
            .bind(&p.code)
            .bind(&p.sku)
            .bind(&p.name)
            .bind(p.stock)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "UPDATE erp_connections SET last_synced_at = strftime('%Y-%m-%dT%H:%M:%SZ','now'), last_sync_error = NULL WHERE id = ?",
        )
        .bind(conn.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        let out: ErpConnection = sqlx::query_as(&format!("{CONN_SELECT} AND c.id = ?"))
            .bind(conn.id)
            .fetch_one(&self.pool)
            .await?;
        self.log(
            who,
            "erp_sync",
            None,
            format!(
                "pulled {} products from {} for {brand}, {} matched to items",
                out.products,
                erp::label(&conn.kind),
                out.matched
            ),
        )
        .await?;
        Ok(out)
    }

    /// Customer invoices from the brand's ERP, read live (nothing is stored).
    pub async fn erp_invoices(
        &self,
        who: &Principal,
        a: &ErpInvoiceArgs,
    ) -> SResult<Vec<erp::Invoice>> {
        who.require(Scope::Read)?;
        if a.from > a.to {
            return invalid("`from` must be on or before `to`");
        }
        let conn = self.erp_conn(&a.brand).await?;
        let read = async { self.erp_client(&conn).await?.invoices(&a.from, &a.to).await }.await;
        read.map_err(|e| provider_error(who, e, &format!("{} invoices", erp::label(&conn.kind))))
    }

    pub async fn erp_links(&self, item_id: i64) -> SResult<Vec<ErpLink>> {
        Ok(sqlx::query_as(
            // The item's own SKU or any of its listings' SKUs (merged items keep the source's
            // listings), ignoring case like `resolve_item` unless another item has the exact SKU.
            "SELECT DISTINCT c.kind AS system, p.code, p.name, p.stock FROM items i
             JOIN erp_connections c ON c.brand_id = i.brand_id AND c.active = 1
             JOIN erp_products p ON p.connection_id = c.id
             WHERE i.id = ? AND (p.sku = i.sku
               OR EXISTS (SELECT 1 FROM listings l WHERE l.item_id = i.id AND l.sku = p.sku)
               OR (NOT EXISTS (SELECT 1 FROM items j WHERE j.brand_id = i.brand_id AND j.sku = p.sku)
                   AND (p.sku = i.sku COLLATE NOCASE
                        OR EXISTS (SELECT 1 FROM listings l WHERE l.item_id = i.id AND l.sku = p.sku COLLATE NOCASE))))
             ORDER BY p.code",
        )
        .bind(item_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Pushes a confirmed or fulfilled sales order to the brand's ERP as an order.
    pub async fn push_so_to_erp(&self, who: &Principal, id: i64) -> SResult<ErpOrder> {
        human(who)?;
        let so = self.sales_order(who, id).await?;
        if !["confirmed", "fulfilled"].contains(&so.status.as_str()) {
            return invalid(format!(
                "SO #{id} is {}; only confirmed sales orders go to the ERP",
                so.status
            ));
        }
        let mut lines = Vec::new();
        for l in &so.lines {
            let Some(price) = l.unit_price else {
                return invalid(format!("{} on SO #{id} has no unit price", l.sku));
            };
            lines.push(OrderLine {
                sku: l.sku.clone(),
                name: l.name.clone(),
                quantity: l.quantity as f64,
                unit_price: price,
            });
        }
        let conn = self.erp_conn(&so.brand).await?;
        // Claim: a new row, a failed one, or a pending one abandoned for 10 minutes, and only while
        // the SO is confirmed/fulfilled. Cancelling refuses once a claim exists, so a push and a
        // cancel can't both win. The claim token stops a superseded push from overwriting the
        // newer one's result.
        let claim: Option<(String,)> = sqlx::query_as(
            "INSERT INTO erp_orders (connection_id, sales_order_id, status, created_by, claim)
             SELECT ?1, ?2, 'pending', ?3, lower(hex(randomblob(16)))
             WHERE EXISTS (SELECT 1 FROM sales_orders WHERE id = ?2 AND status IN ('confirmed', 'fulfilled'))
             ON CONFLICT(connection_id, sales_order_id) DO UPDATE
               SET status = 'pending', error = NULL, created_by = excluded.created_by, claim = excluded.claim,
                   updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
             WHERE erp_orders.status = 'failed'
                OR (erp_orders.status = 'pending'
                    AND erp_orders.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-10 minutes'))
             RETURNING claim",
        )
        .bind(conn.id)
        .bind(id)
        .bind(&who.name)
        .fetch_optional(&self.pool)
        .await?;
        let Some((claim,)) = claim else {
            let current = self.sales_order(who, id).await?;
            if !["confirmed", "fulfilled"].contains(&current.status.as_str()) {
                return invalid(format!(
                    "SO #{id} is {}; only confirmed sales orders go to the ERP",
                    current.status
                ));
            }
            let row = self.erp_order(conn.id, id).await?;
            return match row.status.as_str() {
                "exported" => Ok(row),
                _ => Err(ServiceError::Conflict(format!(
                    "SO #{id} is already being sent to the ERP"
                ))),
            };
        };
        let row = self.erp_order(conn.id, id).await?;
        let order = Order {
            number: format!("SO-{id}"),
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            customer: so.customer.clone(),
            email: so.customer_email.clone(),
            note: match (&so.external_ref, &so.note) {
                (Some(r), Some(n)) => Some(format!("Customer ref {r} · {n}")),
                (Some(r), None) => Some(format!("Customer ref {r}")),
                (None, n) => n.clone(),
            },
            lines,
        };
        let pushed = async {
            let erp = self.erp_client(&conn).await?;
            match erp.find_order(&order.number).await? {
                Some(existing) => Ok(existing),
                None => erp.create_order(&order).await,
            }
        }
        .await;
        let system = erp::label(&conn.kind);
        let (status, external_id, error) = match &pushed {
            Ok(x) => ("exported", Some(x.clone()), None),
            Err(e) => ("failed", None, Some(format!("{e:#}"))),
        };
        let done = sqlx::query(
            "UPDATE erp_orders SET status = ?, external_id = ?, error = ?,
             updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ? AND claim = ?",
        )
        .bind(status)
        .bind(&external_id)
        .bind(&error)
        .bind(row.id)
        .bind(&claim)
        .execute(&self.pool)
        .await?;
        if done.rows_affected() == 0 {
            return Err(ServiceError::Conflict(format!(
                "SO #{id} was taken over by a newer push"
            )));
        }
        let subject = Some(("sales_order", id));
        match pushed {
            Ok(x) => {
                self.log(
                    who,
                    "erp_push",
                    subject,
                    format!("SO-{id} created in {system} as order {x}"),
                )
                .await?;
                self.erp_order(conn.id, id).await
            }
            Err(e) => {
                self.log(
                    who,
                    "erp_push_failed",
                    subject,
                    format!("SO-{id} failed in {system} (error on the ERP status)"),
                )
                .await?;
                Err(ServiceError::Internal(e))
            }
        }
    }

    /// Refuses to cancel an order that is in (or being sent to) the ERP.
    pub(crate) async fn ensure_not_in_erp(&self, id: i64) -> SResult<()> {
        let sent: Option<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT c.kind, o.status, o.external_id FROM erp_orders o JOIN erp_connections c ON c.id = o.connection_id
             WHERE o.sales_order_id = ? AND o.status IN ('exported', 'pending') LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((kind, status, external_id)) = sent else {
            return Ok(());
        };
        let system = erp::label(&kind);
        Err(ServiceError::Conflict(if status == "pending" {
            format!("SO #{id} is being sent to {system}; try again once it finishes")
        } else {
            format!(
                "SO #{id} is in {system} as order {}; cancel it in {system} first",
                external_id.unwrap_or_default()
            )
        }))
    }

    async fn erp_order(&self, conn: i64, id: i64) -> SResult<ErpOrder> {
        Ok(sqlx::query_as(&format!(
            "{ORDER_SELECT} WHERE o.connection_id = ? AND o.sales_order_id = ?"
        ))
        .bind(conn)
        .bind(id)
        .fetch_one(&self.pool)
        .await?)
    }
}
