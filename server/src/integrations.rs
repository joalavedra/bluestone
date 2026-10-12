//! Settings → Integrations: a catalog of connectors, each describing the fields its setup form
//! needs, plus test-before-save and save. Pasted keys are stored encrypted (`secret:<id>`);
//! self-hosters can point at an env var or `nango:<integration>/<connection>` instead.

use crate::accounting::{self, Config};
use crate::accounting_sync::NewAccountingConnection;
use crate::auth::{Principal, Scope};
use crate::credentials::{self, OAUTH, SECRET_PREFIX};
use crate::service::{NewChannel, SResult, Service, ServiceError, invalid};
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Serialize)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    /// `text` or `url`.
    pub kind: &'static str,
    pub required: bool,
    pub placeholder: Option<&'static str>,
    pub default: Option<&'static str>,
    pub help: Option<&'static str>,
}

impl Field {
    fn new(key: &'static str, label: &'static str, kind: &'static str) -> Self {
        Self {
            key,
            label,
            kind,
            required: true,
            placeholder: None,
            default: None,
            help: None,
        }
    }
    fn placeholder(mut self, p: &'static str) -> Self {
        self.placeholder = Some(p);
        self
    }
    fn default(mut self, d: &'static str) -> Self {
        self.default = Some(d);
        self.required = false;
        self
    }
    fn help(mut self, h: &'static str) -> Self {
        self.help = Some(h);
        self
    }
}

#[derive(Debug, Serialize)]
pub struct Credential {
    pub label: &'static str,
    pub help: &'static str,
    /// Can be left empty and connected through the provider's own login after saving (Faire).
    pub oauth: bool,
    /// Example advanced reference shown in the form.
    pub reference_example: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Connector {
    pub id: &'static str,
    pub name: &'static str,
    /// `sales_channel`, `accounting` or `email`.
    pub category: &'static str,
    pub description: &'static str,
    /// `available` or `coming_soon`.
    pub status: &'static str,
    pub fields: Vec<Field>,
    pub credential: Option<Credential>,
}

fn soon(
    id: &'static str,
    name: &'static str,
    category: &'static str,
    description: &'static str,
) -> Connector {
    Connector {
        id,
        name,
        category,
        description,
        status: "coming_soon",
        fields: vec![],
        credential: None,
    }
}

fn channel_name(default: &'static str) -> Field {
    Field::new("name", "Channel name", "text")
        .default(default)
        .help(
            "A label for this store, e.g. shopify-eu. Saving again with the same name updates it.",
        )
}

pub fn catalog() -> Vec<Connector> {
    vec![
        Connector {
            id: "shopify",
            name: "Shopify",
            category: "sales_channel",
            description: "Products, stock per location and orders from a Shopify store; approved changes are pushed back.",
            status: "available",
            fields: vec![
                channel_name("shopify"),
                Field::new("base_url", "Store address", "url")
                    .placeholder("https://your-shop.myshopify.com"),
            ],
            credential: Some(Credential {
                label: "Admin API access token",
                help: "Shopify admin → Settings → Apps → Develop apps → your app → API credentials. Scopes: read/write products, inventory, orders and locations.",
                oauth: false,
                reference_example: "SHOPIFY_TOKEN or nango:shopify/my-brand",
            }),
        },
        Connector {
            id: "prestashop",
            name: "PrestaShop",
            category: "sales_channel",
            description: "Products, stock and orders through the PrestaShop WebService.",
            status: "available",
            fields: vec![
                channel_name("prestashop"),
                Field::new("base_url", "Shop address", "url")
                    .placeholder("https://shop.example.com"),
            ],
            credential: Some(Credential {
                label: "WebService key",
                help: "Back office → Advanced parameters → Webservice. Allow GET/PUT on products, combinations, stock_availables and orders.",
                oauth: false,
                reference_example: "PRESTASHOP_KEY",
            }),
        },
        Connector {
            id: "faire",
            name: "Faire",
            category: "sales_channel",
            description: "Wholesale products, on-hand inventory, wholesale prices and orders.",
            status: "available",
            fields: vec![
                channel_name("faire"),
                Field::new("base_url", "API address", "url").default("https://www.faire.com"),
            ],
            credential: Some(Credential {
                label: "OAuth access token (optional)",
                help: "Leave empty and click Connect Faire after saving to sign in with Faire. The server needs FAIRE_APP_ID and FAIRE_APP_SECRET.",
                oauth: true,
                reference_example: "FAIRE_TOKEN",
            }),
        },
        Connector {
            id: "quickbooks",
            name: "QuickBooks Online",
            category: "accounting",
            description: "Approved purchase orders become supplier bills, confirmed sales orders become invoices.",
            status: "available",
            fields: vec![
                Field::new("realm", "Company ID (realm)", "text").placeholder("9130 3570 1234 567"),
                Field::new("income_account", "Income account ID", "text")
                    .help("Used as the sales income account on items Bluestone creates."),
                Field::new("expense_account", "Expense account ID", "text")
                    .help("Used as the purchase expense account on items Bluestone creates."),
                Field::new("base_url", "API address", "url")
                    .default("https://quickbooks.api.intuit.com")
                    .help("Use https://sandbox-quickbooks.api.intuit.com for a sandbox company."),
            ],
            credential: Some(Credential {
                label: "OAuth access token",
                help: "Pasted tokens expire after an hour; for day-to-day use connect through self-hosted Nango (Advanced).",
                oauth: false,
                reference_example: "nango:quickbooks/my-brand",
            }),
        },
        Connector {
            id: "holded",
            name: "Holded",
            category: "accounting",
            description: "Approved purchase orders become purchases, confirmed sales orders become invoices.",
            status: "available",
            fields: vec![
                Field::new("base_url", "API address", "url").default("https://api.holded.com"),
            ],
            credential: Some(Credential {
                label: "API key",
                help: "Holded → Settings → Developers → New API key.",
                oauth: false,
                reference_example: "HOLDED_KEY or nango:holded/my-brand",
            }),
        },
        soon(
            "amazon",
            "Amazon Seller Central",
            "sales_channel",
            "FBA and merchant-fulfilled stock and orders.",
        ),
        soon(
            "tiktok_shop",
            "TikTok Shop",
            "sales_channel",
            "Products, stock and orders from TikTok Shop.",
        ),
        soon(
            "woocommerce",
            "WooCommerce",
            "sales_channel",
            "Products, stock and orders from a WooCommerce store.",
        ),
        soon("xero", "Xero", "accounting", "Bills and invoices in Xero."),
        soon(
            "gmail",
            "Gmail",
            "email",
            "An order inbox your agent watches; confirmations and invoices sent back to customers.",
        ),
    ]
}

/// What the setup form sends. Exactly one of `secret` (pasted, stored encrypted) or
/// `credential_ref` (env var or `nango:…`) may be set; with neither, the saved credential is kept.
#[derive(Debug, Deserialize)]
pub struct IntegrationInput {
    pub connector: String,
    pub brand: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    pub secret: Option<String>,
    pub credential_ref: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Connection {
    pub connector: String,
    /// `channel` or `accounting`.
    pub kind: &'static str,
    pub id: i64,
    pub brand: String,
    pub label: String,
    /// Non-secret form values, to prefill editing.
    pub fields: BTreeMap<String, String>,
    /// `stored`, `env`, `nango` or `oauth`; never the key itself.
    pub credential_source: &'static str,
    pub last_synced_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct IntegrationsOverview {
    pub connectors: Vec<Connector>,
    pub connections: Vec<Connection>,
}

#[derive(Debug, Serialize)]
pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct Saved {
    /// `channel` or `accounting`.
    pub kind: &'static str,
    pub id: i64,
    /// Saved without a key: the UI starts the provider's OAuth connect next.
    pub needs_oauth: bool,
}

enum Cred {
    Pasted(String),
    Reference(String),
    /// The credential already saved for this connection.
    Existing {
        reference: String,
        channel_id: i64,
    },
    Oauth,
}

fn source(reference: &str) -> &'static str {
    if reference == OAUTH {
        "oauth"
    } else if reference.starts_with(SECRET_PREFIX) {
        "stored"
    } else if reference.starts_with("nango:") {
        "nango"
    } else {
        "env"
    }
}

fn admin_human(who: &Principal) -> SResult<()> {
    who.require(Scope::Admin)?;
    if who.kind != "human" && who.kind != "system" {
        return Err(ServiceError::Forbidden(
            "only human tokens can set up integrations".into(),
        ));
    }
    Ok(())
}

fn connector(id: &str) -> SResult<Connector> {
    let Some(c) = catalog().into_iter().find(|c| c.id == id) else {
        return invalid(format!("unknown connector `{id}`"));
    };
    if c.status != "available" {
        return invalid(format!("{} is coming soon", c.name));
    }
    Ok(c)
}

fn values(c: &Connector, i: &IntegrationInput) -> SResult<BTreeMap<&'static str, String>> {
    if i.brand.trim().is_empty() {
        return invalid("brand is required");
    }
    let mut out = BTreeMap::new();
    for f in &c.fields {
        let v = i
            .fields
            .get(f.key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .or(f.default);
        match v {
            Some(v) => {
                out.insert(f.key, v.to_owned());
            }
            None if f.required => return invalid(format!("{} is required", f.label)),
            None => {}
        }
    }
    Ok(out)
}

fn non_empty(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

impl Service {
    pub async fn integrations(&self, who: &Principal) -> SResult<IntegrationsOverview> {
        who.require(Scope::Read)?;
        type ChannelRow = (
            i64,
            String,
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
        );
        let channels: Vec<ChannelRow> = sqlx::query_as(
            "SELECT c.id, b.name, c.kind, c.name, c.base_url, c.credential_env, c.last_synced_at, c.last_sync_error
             FROM channels c JOIN brands b ON b.id = c.brand_id ORDER BY b.name, c.name",
        )
        .fetch_all(&self.pool)
        .await?;
        type BooksRow = (
            i64,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
            String,
        );
        let books: Vec<BooksRow> = sqlx::query_as(
            "SELECT a.id, b.name, a.kind, a.base_url, a.realm, a.income_account, a.expense_account, a.credential_env
             FROM accounting_connections a JOIN brands b ON b.id = a.brand_id WHERE a.active = 1 ORDER BY b.name",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut connections: Vec<Connection> = channels
            .into_iter()
            .map(
                |(id, brand, kind, name, base_url, cred, synced, error)| Connection {
                    connector: kind,
                    kind: "channel",
                    id,
                    brand,
                    label: name.clone(),
                    fields: BTreeMap::from([("name".into(), name), ("base_url".into(), base_url)]),
                    credential_source: source(&cred),
                    last_synced_at: synced,
                    error,
                },
            )
            .collect();
        connections.extend(books.into_iter().map(
            |(id, brand, kind, base_url, realm, income, expense, cred)| {
                let mut fields = BTreeMap::from([("base_url".to_owned(), base_url)]);
                for (k, v) in [
                    ("realm", realm),
                    ("income_account", income),
                    ("expense_account", expense),
                ] {
                    if let Some(v) = v {
                        fields.insert(k.into(), v);
                    }
                }
                Connection {
                    label: accounting::label(&kind).to_owned(),
                    connector: kind,
                    kind: "accounting",
                    id,
                    brand,
                    fields,
                    credential_source: source(&cred),
                    last_synced_at: None,
                    error: None,
                }
            },
        ));
        Ok(IntegrationsOverview {
            connectors: catalog(),
            connections,
        })
    }

    async fn existing_credential(
        &self,
        c: &Connector,
        brand: &str,
        vals: &BTreeMap<&'static str, String>,
    ) -> SResult<Option<(String, i64)>> {
        Ok(if c.category == "sales_channel" {
            sqlx::query_as(
                "SELECT c.credential_env, c.id FROM channels c JOIN brands b ON b.id = c.brand_id
                 WHERE b.name = ? AND c.name = ? AND c.kind = ?",
            )
            .bind(brand)
            .bind(&vals["name"])
            .bind(c.id)
            .fetch_optional(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT a.credential_env, 0 FROM accounting_connections a JOIN brands b ON b.id = a.brand_id
                 WHERE b.name = ? AND a.kind = ? AND a.active = 1",
            )
            .bind(brand)
            .bind(c.id)
            .fetch_optional(&self.pool)
            .await?
        })
    }

    async fn credential(
        &self,
        c: &Connector,
        i: &IntegrationInput,
        vals: &BTreeMap<&'static str, String>,
    ) -> SResult<Cred> {
        let pasted = non_empty(&i.secret);
        let reference = non_empty(&i.credential_ref);
        if let Some(r) = reference
            && (r == OAUTH || r.starts_with(SECRET_PREFIX))
        {
            return invalid(
                "the reference must be an env var name or nango:<integration>/<connection>",
            );
        }
        Ok(match (pasted, reference) {
            (Some(_), Some(_)) => return invalid("paste a key or give a reference, not both"),
            (Some(p), None) => Cred::Pasted(p.to_owned()),
            (None, Some(r)) => Cred::Reference(r.to_owned()),
            (None, None) => match self.existing_credential(c, i.brand.trim(), vals).await? {
                Some((reference, channel_id)) => Cred::Existing {
                    reference,
                    channel_id,
                },
                None if c.credential.as_ref().is_some_and(|k| k.oauth) => Cred::Oauth,
                None => {
                    let label = c.credential.as_ref().map_or("key", |k| k.label);
                    return invalid(format!(
                        "paste the {label}, or set an env var / Nango reference under Advanced"
                    ));
                }
            },
        })
    }

    /// Tries the connection with a read-only call, without saving anything.
    pub async fn test_integration(
        &self,
        who: &Principal,
        i: &IntegrationInput,
    ) -> SResult<TestResult> {
        admin_human(who)?;
        let c = connector(&i.connector)?;
        let vals = values(&c, i)?;
        let cred = self.credential(&c, i, &vals).await?;
        let probe = async {
            let token = match cred {
                Cred::Pasted(s) => credentials::for_kind(c.id, s)?,
                Cred::Reference(r) => credentials::resolve(&self.pool, 0, c.id, &r).await?,
                Cred::Existing {
                    reference,
                    channel_id,
                } => credentials::resolve(&self.pool, channel_id, c.id, &reference).await?,
                Cred::Oauth => return Ok(None),
            };
            let base_url = vals.get("base_url").cloned().unwrap_or_default();
            let msg = if c.category == "sales_channel" {
                let catalog = crate::connectors::build(c.id, &base_url, token)?
                    .fetch_catalog()
                    .await?;
                format!(
                    "{} listings in {} locations",
                    catalog.listings.len(),
                    catalog.locations.len()
                )
            } else {
                accounting::build(Config {
                    kind: c.id.into(),
                    base_url,
                    realm: vals.get("realm").cloned(),
                    income_account: vals.get("income_account").cloned(),
                    expense_account: vals.get("expense_account").cloned(),
                    credential: token,
                })?
                .check()
                .await?
            };
            anyhow::Ok(Some(msg))
        };
        let res = tokio::time::timeout(Duration::from_secs(30), probe)
            .await
            .unwrap_or_else(|_| Err(anyhow!("no answer within 30 seconds")));
        Ok(match res {
            Ok(Some(msg)) => TestResult {
                ok: true,
                message: format!("{}: {msg}", c.name),
            },
            Ok(None) => TestResult {
                ok: false,
                message: format!("Save, then click Connect {} to sign in", c.name),
            },
            Err(e) => TestResult {
                ok: false,
                message: format!("{e:#}"),
            },
        })
    }

    pub async fn save_integration(&self, who: &Principal, i: &IntegrationInput) -> SResult<Saved> {
        admin_human(who)?;
        let c = connector(&i.connector)?;
        let vals = values(&c, i)?;
        let cred = self.credential(&c, i, &vals).await?;
        let needs_oauth = matches!(cred, Cred::Oauth);
        let credential_env = match cred {
            Cred::Pasted(s) => credentials::store_secret(&self.pool, &s)
                .await
                .map_err(|e| ServiceError::Invalid(format!("could not store the key: {e:#}")))?,
            Cred::Reference(r) | Cred::Existing { reference: r, .. } => r,
            Cred::Oauth => OAUTH.into(),
        };
        let brand = i.brand.trim().to_owned();
        let saved = if c.category == "sales_channel" {
            self.add_channel(
                who,
                &NewChannel {
                    brand,
                    kind: c.id.into(),
                    name: vals["name"].clone(),
                    base_url: vals["base_url"].clone(),
                    credential_env,
                },
            )
            .await
            .map(|id| ("channel", id))
        } else {
            self.set_accounting(
                who,
                &NewAccountingConnection {
                    brand,
                    kind: c.id.into(),
                    base_url: vals.get("base_url").cloned(),
                    realm: vals.get("realm").cloned(),
                    credential_env,
                    income_account: vals.get("income_account").cloned(),
                    expense_account: vals.get("expense_account").cloned(),
                },
            )
            .await
            .map(|id| ("accounting", id))
        };
        // Drops the key just stored if saving failed, and any key this save replaced.
        credentials::prune_secrets(&self.pool).await?;
        let (kind, id) = saved?;
        Ok(Saved {
            kind,
            id,
            needs_oauth,
        })
    }

    /// Records that someone wants a connector that isn't built yet.
    pub async fn request_integration(&self, who: &Principal, id: &str) -> SResult<()> {
        who.require(Scope::Read)?;
        let Some(c) = catalog().into_iter().find(|c| c.id == id) else {
            return invalid(format!("unknown connector `{id}`"));
        };
        self.log(
            who,
            "integration_request",
            None,
            format!("requested the {} connector", c.name),
        )
        .await
    }
}
