use anyhow::Result;
use bluestone::auth::{self, Principal};
use bluestone::mcp::BluestoneMcp;
use bluestone::service::{NewChannel, Service};
use bluestone::{api, db};
use clap::{Parser, Subcommand};
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

#[derive(Parser)]
#[command(
    name = "bluestone",
    about = "Agent-first inventory hub for Shopify and PrestaShop brands"
)]
struct Cli {
    #[arg(
        long,
        env = "BLUESTONE_DATABASE",
        default_value = "sqlite://bluestone.db",
        global = true
    )]
    database: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the REST API (/api) and MCP streamable HTTP endpoint (/mcp).
    Serve {
        #[arg(long, env = "BLUESTONE_ADDR", default_value = "127.0.0.1:8787")]
        addr: String,
        /// Extra hostnames accepted by /mcp (DNS-rebinding protection), comma separated.
        #[arg(long, env = "BLUESTONE_ALLOWED_HOSTS", value_delimiter = ',')]
        allowed_hosts: Vec<String>,
        /// Sync every channel every N seconds (0 = off).
        #[arg(long, env = "BLUESTONE_SYNC_EVERY", default_value_t = 0)]
        sync_every: u64,
    },
    /// Run the MCP server over stdio (for Claude Code). Auth via BLUESTONE_TOKEN.
    Mcp {
        #[arg(long, env = "BLUESTONE_TOKEN", hide_env_values = true)]
        token: String,
    },
    /// Manage API / MCP tokens.
    Token {
        #[command(subcommand)]
        cmd: TokenCmd,
    },
    /// Connect a Shopify or PrestaShop store to a brand.
    Channel {
        #[command(subcommand)]
        cmd: ChannelCmd,
    },
    /// Pull products, stock and orders from every channel (or one).
    Sync {
        #[arg(long)]
        channel: Option<i64>,
    },
}

#[derive(Subcommand)]
enum TokenCmd {
    Create {
        #[arg(long)]
        name: String,
        /// agent or human
        #[arg(long, default_value = "agent")]
        kind: String,
        /// Comma separated: read,organise,propose,approve,admin
        #[arg(long, default_value = "read,organise,propose")]
        scopes: String,
    },
    List,
}

#[derive(Subcommand)]
enum ChannelCmd {
    Add {
        #[arg(long)]
        brand: String,
        /// shopify or prestashop
        #[arg(long)]
        kind: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        base_url: String,
        /// Env var that holds the access token / WebService key.
        #[arg(long)]
        credential_env: String,
    },
    List,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let pool = db::connect(&cli.database).await?;
    let svc = Arc::new(Service::new(pool));

    match cli.cmd {
        Cmd::Serve {
            addr,
            allowed_hosts,
            sync_every,
        } => serve(svc, &addr, allowed_hosts, sync_every).await?,
        Cmd::Mcp { token } => {
            let principal = auth::authenticate(&svc.pool, &token)
                .await
                .ok_or_else(|| anyhow::anyhow!("BLUESTONE_TOKEN is not a valid token"))?;
            tracing::info!(token = %principal.name, "mcp stdio session");
            BluestoneMcp::new(svc, Some(principal))
                .serve(rmcp::transport::stdio())
                .await?
                .waiting()
                .await?;
        }
        Cmd::Token {
            cmd: TokenCmd::Create { name, kind, scopes },
        } => {
            let scopes = auth::parse_scopes(&scopes)?;
            let token = auth::create_token(&svc.pool, &name, &kind, &scopes).await?;
            eprintln!("created {kind} token `{name}` — store it now, it is not shown again:");
            println!("{token}");
        }
        Cmd::Token {
            cmd: TokenCmd::List,
        } => {
            let rows: Vec<(String, String, String, Option<String>)> =
                sqlx::query_as("SELECT name, kind, scopes, last_used_at FROM tokens ORDER BY name")
                    .fetch_all(&svc.pool)
                    .await?;
            for (name, kind, scopes, used) in rows {
                println!(
                    "{name}\t{kind}\t{scopes}\tlast used {}",
                    used.unwrap_or_else(|| "never".into())
                );
            }
        }
        Cmd::Channel {
            cmd:
                ChannelCmd::Add {
                    brand,
                    kind,
                    name,
                    base_url,
                    credential_env,
                },
        } => {
            let id = svc
                .add_channel(
                    &Principal::system(),
                    &NewChannel {
                        brand,
                        kind,
                        name,
                        base_url,
                        credential_env,
                    },
                )
                .await?;
            println!("channel {id}");
        }
        Cmd::Channel {
            cmd: ChannelCmd::List,
        } => {
            for (_, c) in svc.channels().await? {
                println!(
                    "{}\t{}\t{}\t{}\tsynced {}",
                    c.id,
                    c.kind,
                    c.name,
                    c.base_url,
                    c.last_synced_at.unwrap_or_else(|| "never".into())
                );
            }
        }
        Cmd::Sync { channel } => {
            let who = Principal::system();
            let reports = match channel {
                Some(id) => vec![svc.sync_channel(&who, id).await?],
                None => svc.sync_all(&who).await?,
            };
            for r in reports {
                println!(
                    "{}: {} listings, {} new items, {} order lines",
                    r.channel, r.listings, r.new_items, r.order_lines
                );
            }
        }
    }
    Ok(())
}

async fn serve(
    svc: Arc<Service>,
    addr: &str,
    extra_hosts: Vec<String>,
    sync_every: u64,
) -> Result<()> {
    let mut config = StreamableHttpServerConfig::default();
    config
        .allowed_hosts
        .extend(extra_hosts.into_iter().filter(|h| !h.is_empty()));
    let mcp_svc = svc.clone();
    let mcp = StreamableHttpService::new(
        move || Ok(BluestoneMcp::new(mcp_svc.clone(), None)),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let app = axum::Router::new()
        .route("/healthz", axum::routing::get(|| async { "ok" }))
        .nest("/api", api::router(svc.clone()))
        .route_service("/mcp", mcp)
        .layer(axum::middleware::from_fn_with_state(svc.clone(), mcp_auth))
        .layer(CorsLayer::permissive());

    if sync_every > 0 {
        let svc = svc.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(sync_every));
            loop {
                tick.tick().await;
                if let Err(e) = svc.sync_all(&Principal::system()).await {
                    tracing::warn!(error = %e, "scheduled sync failed");
                }
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("bluestone listening on http://{addr} (REST /api, MCP /mcp)");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Bearer auth for /mcp only; /api has its own layer and /healthz is open.
async fn mcp_auth(
    axum::extract::State(svc): axum::extract::State<Arc<Service>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if req.uri().path() != "/mcp" {
        return next.run(req).await;
    }
    api::require_token(axum::extract::State(svc), req, next).await
}
