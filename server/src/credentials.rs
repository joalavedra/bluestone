//! Resolves a channel's `credential_env` reference into the secret its connector needs:
//! - `NAME` or `env:NAME`: an environment variable (the default; nothing stored);
//! - `nango:<integration>/<connection>`: a self-hosted Nango instance (`NANGO_HOST`, `NANGO_SECRET_KEY`)
//!   that owns the OAuth app and token refresh;
//! - `oauth`: a token from Bluestone's own connect flow (Faire, which Nango does not support),
//!   encrypted at rest with `BLUESTONE_SECRET_KEY`.

use aes_gcm::aead::{Aead, AeadCore, OsRng};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

pub const OAUTH: &str = "oauth";
const FAIRE_SCOPES: [&str; 6] = [
    "READ_BRAND",
    "READ_PRODUCTS",
    "WRITE_PRODUCTS",
    "READ_ORDERS",
    "READ_INVENTORIES",
    "WRITE_INVENTORIES",
];

fn env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow!("env var {name} is not set"))
}

fn cipher() -> Result<Aes256Gcm> {
    let key = env("BLUESTONE_SECRET_KEY").context("needed to store or read OAuth tokens")?;
    let k = Sha256::digest(key.as_bytes()).to_vec();
    Aes256Gcm::new_from_slice(&k).map_err(|e| anyhow!("{e}"))
}

pub async fn store(pool: &SqlitePool, channel_id: i64, token: &str) -> Result<()> {
    let c = cipher()?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = c
        .encrypt(&nonce, token.as_bytes())
        .map_err(|_| anyhow!("could not encrypt token"))?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO channel_tokens (channel_id, nonce, ciphertext) VALUES (?, ?, ?)
         ON CONFLICT (channel_id) DO UPDATE SET nonce = excluded.nonce, ciphertext = excluded.ciphertext,
           updated_at = strftime('%Y-%m-%dT%H:%M:%SZ','now')",
    )
    .bind(channel_id)
    .bind(hex::encode(nonce))
    .bind(hex::encode(ct))
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE channels SET credential_env = ?, last_sync_error = NULL WHERE id = ?")
        .bind(OAUTH)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

async fn stored(pool: &SqlitePool, channel_id: i64) -> Result<String> {
    let (nonce, ct): (String, String) =
        sqlx::query_as("SELECT nonce, ciphertext FROM channel_tokens WHERE channel_id = ?")
            .bind(channel_id)
            .fetch_optional(pool)
            .await?
            .context("channel is not connected yet; run its Connect flow")?;
    let nonce = hex::decode(nonce)?;
    let plain = cipher()?
        .decrypt(Nonce::from_slice(&nonce), hex::decode(ct)?.as_slice())
        .map_err(|_| anyhow!("stored token cannot be decrypted (BLUESTONE_SECRET_KEY changed?)"))?;
    Ok(String::from_utf8(plain)?)
}

async fn nango(spec: &str) -> Result<String> {
    let (integration, connection) = spec
        .split_once('/')
        .context("expected nango:<integration>/<connection>")?;
    let host = std::env::var("NANGO_HOST").unwrap_or_else(|_| "http://localhost:3003".into());
    let mut url = reqwest::Url::parse(host.trim_end_matches('/'))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("NANGO_HOST is not a base URL"))?
        .push("connections")
        .push(connection);
    let v: Value = reqwest::Client::new()
        .get(url)
        .query(&[("provider_config_key", integration)])
        .bearer_auth(env("NANGO_SECRET_KEY")?)
        .send()
        .await?
        .error_for_status()
        .context("Nango connection lookup failed")?
        .json()
        .await?;
    let c = &v["credentials"];
    c["access_token"]
        .as_str()
        .or(c["apiKey"].as_str())
        .map(str::to_owned)
        .context("Nango connection has no access_token or apiKey")
}

/// Faire needs the app credentials next to the user's OAuth token.
fn faire_app_credentials() -> Result<String> {
    let raw = format!("{}:{}", env("FAIRE_APP_ID")?, env("FAIRE_APP_SECRET")?);
    Ok(base64::engine::general_purpose::STANDARD.encode(raw))
}

pub async fn resolve(
    pool: &SqlitePool,
    channel_id: i64,
    kind: &str,
    reference: &str,
) -> Result<String> {
    let token = if reference == OAUTH {
        stored(pool, channel_id).await?
    } else if let Some(spec) = reference.strip_prefix("nango:") {
        nango(spec).await?
    } else {
        return env(reference.strip_prefix("env:").unwrap_or(reference));
    };
    if kind == "faire" {
        Ok(format!("{}:{token}", faire_app_credentials()?))
    } else {
        Ok(token)
    }
}

fn faire_redirect_url() -> String {
    let base =
        std::env::var("BLUESTONE_PUBLIC_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".into());
    format!("{}/oauth/faire/callback", base.trim_end_matches('/'))
}

pub async fn faire_authorize_url(pool: &SqlitePool, channel_id: i64) -> Result<String> {
    sqlx::query(
        "DELETE FROM oauth_states WHERE created_at < strftime('%Y-%m-%dT%H:%M:%SZ','now','-15 minutes')",
    )
    .execute(pool)
    .await?;
    let state = uuid::Uuid::new_v4().simple().to_string();
    sqlx::query("INSERT INTO oauth_states (state, channel_id) VALUES (?, ?)")
        .bind(&state)
        .bind(channel_id)
        .execute(pool)
        .await?;
    let base = std::env::var("FAIRE_OAUTH_URL").unwrap_or_else(|_| "https://faire.com".into());
    let mut params = vec![
        ("applicationId", env("FAIRE_APP_ID")?),
        ("state", state),
        ("redirectUrl", faire_redirect_url()),
    ];
    params.extend(FAIRE_SCOPES.iter().map(|s| ("scope", s.to_string())));
    Ok(reqwest::Url::parse_with_params(
        &format!("{}/oauth2/authorize", base.trim_end_matches('/')),
        &params,
    )?
    .to_string())
}

/// Redeems the authorization code. The state must be < 15 minutes old; it is consumed only once
/// the exchange succeeds, so a transient Faire error can be retried.
pub async fn faire_callback(pool: &SqlitePool, state: &str, code: &str) -> Result<i64> {
    let (channel_id,): (i64,) = sqlx::query_as(
        "SELECT channel_id FROM oauth_states WHERE state = ? AND created_at >= strftime('%Y-%m-%dT%H:%M:%SZ','now','-15 minutes')",
    )
    .bind(state)
    .fetch_optional(pool)
    .await?
    .context("unknown or expired OAuth state; start the connection again")?;
    let token_url = std::env::var("FAIRE_TOKEN_URL")
        .unwrap_or_else(|_| "https://www.faire.com/api/external-api-oauth2/token".into());
    let (id, secret) = (env("FAIRE_APP_ID")?, env("FAIRE_APP_SECRET")?);
    // Faire's docs show both camelCase (field table) and snake_case (sample) names; send both.
    let body = json!({
        "applicationId": id, "applicationSecret": secret, "scope": FAIRE_SCOPES,
        "grantType": "AUTHORIZATION_CODE", "redirectUrl": faire_redirect_url(), "authorizationCode": code,
        "application_token": id, "application_secret": secret, "grant_type": "AUTHORIZATION_CODE",
        "redirect_url": faire_redirect_url(), "authorization_code": code,
    });
    let v: Value = reqwest::Client::new()
        .post(token_url)
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .context("Faire token exchange failed")?
        .json()
        .await?;
    let token = v["access_token"]
        .as_str()
        .context("Faire returned no access_token")?;
    let consumed = sqlx::query("DELETE FROM oauth_states WHERE state = ?")
        .bind(state)
        .execute(pool)
        .await?;
    if consumed.rows_affected() != 1 {
        return Err(anyhow!("OAuth state was already used"));
    }
    store(pool, channel_id, token).await?;
    Ok(channel_id)
}
