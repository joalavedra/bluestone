use anyhow::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use sqlx::{Executor, Row};
use std::str::FromStr;

pub async fn connect(url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    widen_channel_kinds(&pool).await?;
    Ok(pool)
}

/// Rebuilds `channels` so its `kind` CHECK allows every connector. SQLite cannot alter a CHECK and
/// the swap needs foreign keys off, which sqlx migrations (always transactional) cannot do.
async fn widen_channel_kinds(pool: &SqlitePool) -> Result<()> {
    let sql: String =
        sqlx::query("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'channels'")
            .fetch_one(pool)
            .await?
            .get(0);
    if sql.contains("'faire'") {
        return Ok(());
    }
    let mut conn = pool.acquire().await?;
    conn.execute("PRAGMA foreign_keys = OFF").await?;
    let res = conn
        .execute(
            "BEGIN;
             CREATE TABLE channels_new (
               id INTEGER PRIMARY KEY,
               brand_id INTEGER NOT NULL REFERENCES brands(id),
               kind TEXT NOT NULL CHECK (kind IN ('shopify','prestashop','faire')),
               name TEXT NOT NULL,
               base_url TEXT NOT NULL,
               credential_env TEXT NOT NULL,
               last_synced_at TEXT,
               last_sync_error TEXT,
               created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
               UNIQUE (brand_id, name)
             );
             INSERT INTO channels_new SELECT id, brand_id, kind, name, base_url, credential_env, last_synced_at, last_sync_error, created_at FROM channels;
             DROP TABLE channels;
             ALTER TABLE channels_new RENAME TO channels;
             COMMIT;",
        )
        .await;
    if res.is_err() {
        let _ = conn.execute("ROLLBACK").await;
    }
    conn.execute("PRAGMA foreign_keys = ON").await?;
    res?;
    let broken = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *conn)
        .await?;
    anyhow::ensure!(
        broken.is_empty(),
        "foreign key check failed after rebuilding channels"
    );
    Ok(())
}
