# Bluestone

Agent-first inventory hub for Shopify and PrestaShop brands. Your agent (Claude Code or a custom harness) works through **MCP**; you see, organise and approve in the **web UI**. Stores stay the source of truth — Bluestone mirrors them and pushes back only changes a human approved.

- `server/` — Rust (axum, sqlx/SQLite, rmcp). One binary: REST `/api`, MCP `/mcp` (streamable HTTP) and `bluestone mcp` (stdio).
- `web/` — TanStack Start + Router + Query, shadcn/ui, Recharts. Convexity-inspired.
- `dev/` — mock Shopify dev shop, PrestaShop 8.2 docker compose, smoke test.
- `docs/` — [SPEC](docs/SPEC.md), [FEATURES](docs/FEATURES.md) (mapped vs Convexity), [ROADMAP](docs/ROADMAP.md).

## Quick start (dev shops)

```bash
cd server && cargo build
export BLUESTONE_DATABASE=sqlite://$PWD/bluestone.db SHOPIFY_DEV_TOKEN=shpat_dev_mock

# 1. Shopify dev shop (mock Admin GraphQL API, seeded "Northwind Coffee")
target/debug/mock_shopify &                      # http://127.0.0.1:8788

# 2. Optional: PrestaShop 8.2 dev shop
docker compose -f ../dev/docker-compose.yml up -d  # wait ~2 min for install
export PRESTASHOP_DEV_KEY=$(../dev/prestashop-key.sh)

# Optional: Faire wholesale (mock External API v2, same SKUs as the Shopify mock)
target/debug/mock_faire &                        # http://127.0.0.1:8789
export FAIRE_DEV_CREDENTIAL="ZGV2OmRldg==:faire_dev_mock"

# 3. Wire channels, tokens, sync
target/debug/bluestone channel add --brand "Northwind Coffee" --kind shopify --name shopify-dev \
  --base-url http://127.0.0.1:8788 --credential-env SHOPIFY_DEV_TOKEN
target/debug/bluestone channel add --brand "Northwind Coffee" --kind prestashop --name presta-dev \
  --base-url http://localhost:8080 --credential-env PRESTASHOP_DEV_KEY
target/debug/bluestone channel add --brand "Northwind Coffee" --kind faire --name faire-wholesale \
  --base-url http://127.0.0.1:8789 --credential-env FAIRE_DEV_CREDENTIAL
target/debug/bluestone token create --name joan --kind human --scopes admin   # paste into the UI
target/debug/bluestone token create --name claude-code                        # agent: read,organise,propose
target/debug/bluestone sync
target/debug/bluestone serve &                   # http://127.0.0.1:8787

# 4. UI
cd ../web && npm install && npm run dev          # http://localhost:3000 (proxies /api)
```

## Connect agents

Claude Code (stdio):

```bash
claude mcp add bluestone --env BLUESTONE_TOKEN=bst_… --env BLUESTONE_DATABASE=sqlite:///abs/path/bluestone.db \
  -- /abs/path/server/target/debug/bluestone mcp
```

Custom agent (streamable HTTP): `POST http://127.0.0.1:8787/mcp` with `Authorization: Bearer bst_…`. Set `BLUESTONE_ALLOWED_HOSTS` when serving on a non-loopback host.

Agent tokens can read, organise (tags, suppliers, reorder settings, notes, merges) and **propose** stock/price/status changes. Only human tokens can approve, from the Approvals page.

## Real Shopify store

Create a dev store in your Partner dashboard → custom app with `read_products, write_products, read_inventory, write_inventory, read_locations, read_orders` → `bluestone channel add --kind shopify --base-url https://<shop>.myshopify.com --credential-env SHOPIFY_TOKEN`.

## Real Faire brand

Register an app at developers.faire.com, run the OAuth flow to get an access token for the brand, then `export FAIRE_TOKEN="<base64 applicationId:applicationSecret>:<access token>"` and `bluestone channel add --kind faire --base-url https://www.faire.com --credential-env FAIRE_TOKEN`. Bluestone tracks the **wholesale** price for Faire listings.

## Checks

```bash
cd server && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test && ../dev/smoke.sh && ../dev/smoke-faire.sh
cd web && npx tsr generate && npm run typecheck && npm run lint && npm run build
```
