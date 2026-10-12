//! Pushes approved purchase orders (bills) and confirmed sales orders (invoices) to the brand's
//! accounting system. Human/system only: agents can read the books status but never write to it.

use crate::accounting::{self, Config, Doc, Line, Party};
use crate::auth::{Principal, Scope};
use crate::service::{SResult, Service, ServiceError, invalid};
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AccountingConnection {
    pub id: i64,
    pub brand: String,
    pub kind: String,
    pub base_url: String,
    pub realm: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AccountingDocument {
    pub id: i64,
    pub brand: String,
    /// `quickbooks` or `holded`.
    pub system: String,
    /// `purchase_order` (exported as a bill) or `sales_order` (exported as an invoice).
    pub source: String,
    pub source_id: i64,
    /// `pending`, `exported` or `failed`.
    pub status: String,
    pub external_id: Option<String>,
    pub error: Option<String>,
    pub created_by: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct AccountingOverview {
    pub connections: Vec<AccountingConnection>,
    pub documents: Vec<AccountingDocument>,
}

#[derive(Debug, Deserialize)]
pub struct NewAccountingConnection {
    pub brand: String,
    pub kind: String,
    pub base_url: Option<String>,
    pub realm: Option<String>,
    pub credential_env: String,
    pub income_account: Option<String>,
    pub expense_account: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ConnRow {
    id: i64,
    kind: String,
    base_url: String,
    realm: Option<String>,
    credential_env: String,
    income_account: Option<String>,
    expense_account: Option<String>,
}

const DOC_SELECT: &str = "SELECT d.id, b.name AS brand, c.kind AS system, d.source, d.source_id, d.status,
  d.external_id, d.error, d.created_by, d.updated_at
FROM accounting_documents d JOIN accounting_connections c ON c.id = d.connection_id JOIN brands b ON b.id = c.brand_id";

fn human(who: &Principal) -> SResult<()> {
    who.require(Scope::Approve)?;
    if who.kind != "human" && who.kind != "system" {
        return Err(ServiceError::Forbidden(
            "only human tokens can write to the accounting system".into(),
        ));
    }
    Ok(())
}

struct Export {
    brand: String,
    source: &'static str,
    source_id: i64,
    party: Party,
    contact: String,
    email: Option<String>,
    number: String,
    note: Option<String>,
    /// (sku, name, quantity, unit price)
    lines: Vec<(String, String, i64, f64)>,
}

impl Service {
    /// Connects (or replaces) the brand's accounting system. Documents already exported to the
    /// previous system stay recorded against it.
    pub async fn set_accounting(
        &self,
        who: &Principal,
        c: &NewAccountingConnection,
    ) -> SResult<i64> {
        who.require(Scope::Admin)?;
        human(who)?;
        let Some(default_url) = accounting::default_base_url(&c.kind) else {
            return invalid(format!(
                "accounting kind must be one of {}",
                accounting::KINDS.join(", ")
            ));
        };
        if c.credential_env == crate::credentials::OAUTH {
            return invalid(
                "accounting credentials must be an env var or nango:<integration>/<connection>",
            );
        }
        if c.kind == "quickbooks"
            && (c.realm.is_none() || c.income_account.is_none() || c.expense_account.is_none())
        {
            return invalid("QuickBooks needs realm, income_account and expense_account");
        }
        let (brand_id,): (i64,) = sqlx::query_as(
            "INSERT INTO brands (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name = excluded.name RETURNING id",
        )
        .bind(&c.brand)
        .fetch_one(&self.pool)
        .await?;
        let base_url = c.base_url.clone().unwrap_or_else(|| default_url.into());
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE accounting_connections SET active = 0 WHERE brand_id = ?")
            .bind(brand_id)
            .execute(&mut *tx)
            .await?;
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO accounting_connections (brand_id, kind, base_url, realm, credential_env, income_account, expense_account)
             VALUES (?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(brand_id)
        .bind(&c.kind)
        .bind(&base_url)
        .bind(&c.realm)
        .bind(&c.credential_env)
        .bind(&c.income_account)
        .bind(&c.expense_account)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.log(
            who,
            "accounting_connect",
            None,
            format!("{} now books to {} ({base_url})", c.brand, c.kind),
        )
        .await?;
        Ok(id)
    }

    pub async fn accounting(&self, who: &Principal) -> SResult<AccountingOverview> {
        who.require(Scope::Read)?;
        let connections = sqlx::query_as(
            "SELECT c.id, b.name AS brand, c.kind, c.base_url, c.realm
             FROM accounting_connections c JOIN brands b ON b.id = c.brand_id WHERE c.active = 1 ORDER BY b.name",
        )
        .fetch_all(&self.pool)
        .await?;
        let documents = sqlx::query_as(&format!(
            "{DOC_SELECT} WHERE c.active = 1 ORDER BY d.updated_at DESC, d.id DESC LIMIT 500"
        ))
        .fetch_all(&self.pool)
        .await?;
        Ok(AccountingOverview {
            connections,
            documents,
        })
    }

    /// Exports an approved PO as a supplier bill for the ordered quantities and unit costs.
    pub async fn export_po(&self, who: &Principal, id: i64) -> SResult<AccountingDocument> {
        human(who)?;
        let po = self.purchase_order(who, id).await?;
        if !["approved", "sent", "partial", "received"].contains(&po.status.as_str()) {
            return invalid(format!(
                "PO #{id} is {}; only approved purchase orders can be billed",
                po.status
            ));
        }
        let Some(supplier) = po.supplier.clone() else {
            return invalid(format!("PO #{id} has no supplier to bill"));
        };
        let mut lines = Vec::new();
        for l in &po.lines {
            let Some(cost) = l.unit_cost else {
                return invalid(format!("{} on PO #{id} has no unit cost", l.sku));
            };
            lines.push((l.sku.clone(), l.name.clone(), l.quantity, cost));
        }
        self.export(
            who,
            Export {
                brand: po.brand,
                source: "purchase_order",
                source_id: id,
                party: Party::Supplier,
                contact: supplier,
                email: po.supplier_email,
                number: format!("PO-{id}"),
                note: po.note,
                lines,
            },
        )
        .await
    }

    /// Exports a confirmed or fulfilled sales order as a customer invoice.
    pub async fn export_so(&self, who: &Principal, id: i64) -> SResult<AccountingDocument> {
        human(who)?;
        let so = self.sales_order(who, id).await?;
        if !["confirmed", "fulfilled"].contains(&so.status.as_str()) {
            return invalid(format!(
                "SO #{id} is {}; only confirmed sales orders can be invoiced",
                so.status
            ));
        }
        let mut lines = Vec::new();
        for l in &so.lines {
            let Some(price) = l.unit_price else {
                return invalid(format!("{} on SO #{id} has no unit price", l.sku));
            };
            lines.push((l.sku.clone(), l.name.clone(), l.quantity, price));
        }
        let note = match (&so.external_ref, &so.note) {
            (Some(r), Some(n)) => Some(format!("Customer ref {r} · {n}")),
            (Some(r), None) => Some(format!("Customer ref {r}")),
            (None, n) => n.clone(),
        };
        self.export(
            who,
            Export {
                brand: so.brand,
                source: "sales_order",
                source_id: id,
                party: Party::Customer,
                contact: so.customer,
                email: so.customer_email,
                number: format!("SO-{id}"),
                note,
                lines,
            },
        )
        .await
    }

    async fn export(&self, who: &Principal, e: Export) -> SResult<AccountingDocument> {
        let conn: ConnRow = sqlx::query_as(
            "SELECT c.id, c.kind, c.base_url, c.realm, c.credential_env, c.income_account, c.expense_account
             FROM accounting_connections c JOIN brands b ON b.id = c.brand_id WHERE b.name = ? AND c.active = 1",
        )
        .bind(&e.brand)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            ServiceError::Invalid(format!(
                "{} has no accounting system; connect one with `bluestone accounting set`",
                e.brand
            ))
        })?;
        // Claim the document: a new row, a failed one, or a pending one abandoned for 10 minutes.
        let claimed = sqlx::query(
            "INSERT INTO accounting_documents (connection_id, source, source_id, status, created_by)
             VALUES (?, ?, ?, 'pending', ?)
             ON CONFLICT(connection_id, source, source_id) DO UPDATE
               SET status = 'pending', error = NULL, created_by = excluded.created_by,
                   updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')
             WHERE accounting_documents.status = 'failed'
                OR (accounting_documents.status = 'pending'
                    AND accounting_documents.updated_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-10 minutes'))",
        )
        .bind(conn.id)
        .bind(e.source)
        .bind(e.source_id)
        .bind(&who.name)
        .execute(&self.pool)
        .await?;
        let doc = self.document(conn.id, e.source, e.source_id).await?;
        if claimed.rows_affected() == 0 {
            return match doc.status.as_str() {
                "exported" => Ok(doc),
                _ => Err(ServiceError::Conflict(format!(
                    "{} #{} is already being exported",
                    e.source.replace('_', " "),
                    e.source_id
                ))),
            };
        }
        let what = if e.party == Party::Supplier {
            "bill"
        } else {
            "invoice"
        };
        let subject = Some((e.source, e.source_id));
        match self.push(&conn, &e).await {
            Ok(external_id) => {
                sqlx::query(
                    "UPDATE accounting_documents SET status = 'exported', external_id = ?,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ?",
                )
                .bind(&external_id)
                .bind(doc.id)
                .execute(&self.pool)
                .await?;
                self.log(
                    who,
                    "accounting_export",
                    subject,
                    format!("{} {what} {external_id} created in {}", e.number, conn.kind),
                )
                .await?;
                self.document(conn.id, e.source, e.source_id).await
            }
            Err(err) => {
                let msg = format!("{err:#}");
                sqlx::query(
                    "UPDATE accounting_documents SET status = 'failed', error = ?,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE id = ?",
                )
                .bind(&msg)
                .bind(doc.id)
                .execute(&self.pool)
                .await?;
                self.log(
                    who,
                    "accounting_export_failed",
                    subject,
                    format!("{} {what} failed in {}: {msg}", e.number, conn.kind),
                )
                .await?;
                Err(ServiceError::Internal(err))
            }
        }
    }

    async fn document(&self, conn: i64, source: &str, id: i64) -> SResult<AccountingDocument> {
        Ok(sqlx::query_as(&format!(
            "{DOC_SELECT} WHERE d.connection_id = ? AND d.source = ? AND d.source_id = ?"
        ))
        .bind(conn)
        .bind(source)
        .bind(id)
        .fetch_one(&self.pool)
        .await?)
    }

    async fn push(&self, conn: &ConnRow, e: &Export) -> anyhow::Result<String> {
        let credential =
            crate::credentials::resolve(&self.pool, 0, &conn.kind, &conn.credential_env)
                .await
                .context("accounting credential")?;
        let books = accounting::build(Config {
            kind: conn.kind.clone(),
            base_url: conn.base_url.clone(),
            realm: conn.realm.clone(),
            income_account: conn.income_account.clone(),
            expense_account: conn.expense_account.clone(),
            credential,
        })?;
        let contact = self
            .linked(conn.id, e.party.as_str(), &e.contact, || {
                books.contact(e.party, &e.contact, e.email.as_deref())
            })
            .await?;
        let mut lines = Vec::new();
        for (sku, name, qty, price) in &e.lines {
            let item = self
                .linked(conn.id, "item", sku, || books.item(sku, name))
                .await?;
            lines.push(Line {
                item,
                sku: sku.clone(),
                name: name.clone(),
                quantity: *qty as f64,
                unit_price: *price,
            });
        }
        let doc = Doc {
            contact,
            number: e.number.clone(),
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            note: e.note.clone(),
            lines,
        };
        match e.party {
            Party::Supplier => books.create_bill(&doc).await,
            Party::Customer => books.create_invoice(&doc).await,
        }
    }

    /// Returns the cached accounting id for a contact/item, looking it up remotely once.
    async fn linked<F, Fut>(
        &self,
        conn: i64,
        entity: &str,
        key: &str,
        lookup: F,
    ) -> anyhow::Result<String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<String>>,
    {
        let cached: Option<(String,)> = sqlx::query_as(
            "SELECT external_id FROM accounting_links WHERE connection_id = ? AND entity = ? AND local_key = ?",
        )
        .bind(conn)
        .bind(entity)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((id,)) = cached {
            return Ok(id);
        }
        let id = lookup().await?;
        sqlx::query(
            "INSERT OR REPLACE INTO accounting_links (connection_id, entity, local_key, external_id) VALUES (?, ?, ?, ?)",
        )
        .bind(conn)
        .bind(entity)
        .bind(key)
        .bind(&id)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }
}
