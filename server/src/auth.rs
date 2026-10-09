use anyhow::{Result, bail};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Organise,
    Propose,
    Approve,
    Admin,
}

impl Scope {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "read" => Self::Read,
            "organise" => Self::Organise,
            "propose" => Self::Propose,
            "approve" => Self::Approve,
            "admin" => Self::Admin,
            other => bail!("unknown scope `{other}` (read|organise|propose|approve|admin)"),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Organise => "organise",
            Self::Propose => "propose",
            Self::Approve => "approve",
            Self::Admin => "admin",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub name: String,
    pub kind: String,
    pub scopes: Vec<Scope>,
}

impl Principal {
    pub fn system() -> Self {
        Self {
            name: "system".into(),
            kind: "system".into(),
            scopes: vec![
                Scope::Read,
                Scope::Organise,
                Scope::Propose,
                Scope::Approve,
                Scope::Admin,
            ],
        }
    }

    pub fn has(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope) || self.scopes.contains(&Scope::Admin)
    }

    pub fn require(&self, scope: Scope) -> Result<(), crate::service::ServiceError> {
        if self.has(scope) {
            Ok(())
        } else {
            Err(crate::service::ServiceError::Forbidden(format!(
                "token `{}` lacks the `{}` scope",
                self.name,
                scope.as_str()
            )))
        }
    }
}

pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn parse_scopes(raw: &str) -> Result<Vec<Scope>> {
    raw.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(Scope::parse)
        .collect()
}

pub async fn create_token(
    pool: &SqlitePool,
    name: &str,
    kind: &str,
    scopes: &[Scope],
) -> Result<String> {
    if kind != "agent" && kind != "human" {
        bail!("kind must be `agent` or `human`");
    }
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = format!("bst_{}", hex::encode(bytes));
    let scopes = scopes
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(",");
    sqlx::query("INSERT INTO tokens (name, kind, token_hash, scopes) VALUES (?, ?, ?, ?)")
        .bind(name)
        .bind(kind)
        .bind(hash_token(&token))
        .bind(scopes)
        .execute(pool)
        .await?;
    Ok(token)
}

pub async fn authenticate(pool: &SqlitePool, token: &str) -> Option<Principal> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT name, kind, scopes FROM tokens WHERE token_hash = ?")
            .bind(hash_token(token))
            .fetch_optional(pool)
            .await
            .ok()?;
    let (name, kind, scopes) = row?;
    let _ = sqlx::query(
        "UPDATE tokens SET last_used_at = strftime('%Y-%m-%dT%H:%M:%SZ','now') WHERE name = ?",
    )
    .bind(&name)
    .execute(pool)
    .await;
    Some(Principal {
        name,
        kind,
        scopes: parse_scopes(&scopes).ok()?,
    })
}

pub fn bearer(headers: &http::HeaderMap) -> Option<&str> {
    headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}
