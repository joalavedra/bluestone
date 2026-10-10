# Bluestone — roadmap

Ordering principle: make the agent useful on real data first, then make it safe to let it do more, then make Bluestone the place stock is decided.

## Phase 0 — Visibility hub (this PR) ✅

Goal: Joan's agent and Joan see the same inventory across Shopify and PrestaShop dev shops, organise it, and push approved changes.

- Rust server: SQLite, REST API, MCP (stdio + streamable HTTP), scoped tokens.
- Connectors: Shopify Admin GraphQL, PrestaShop WebService; manual / scheduled sync.
- Organise: tags, suppliers, reorder settings, notes, merges.
- Proposals: stock / price / status with diff, warnings, human approval, compare-and-set writes.
- UI: Overview, Inventory, Item, Approvals, Reorder, Activity, Channels.
- Dev shops: mock Shopify (seeded), PrestaShop 8.2 in Docker.

Exit criteria: from Claude Code, "what's running low at Northwind?" → tag + propose restock → approve in UI → quantity changed in the store → logged.

## Phase 1 — Real stores & safer autonomy

Why next: real data surfaces matching/velocity issues early; policy rules cut approval fatigue, which is what makes an agent actually save time.

- Real Shopify dev store / custom app; first real PrestaShop shop (read-only key first).
- Webhooks for orders and inventory → near real-time.
- Policy rules (Valet-style): auto-apply under limits, per-agent budgets, quiet hours.
- ✅ Purchase orders: drafts per supplier from the reorder list or by the agent; human approves, sends, receives. PDF / email export still to do.
- Saved views shared by UI and agent; stock history chart.
- Postgres option for hosted deployments; Docker image + compose.

## Phase 2 — Bluestone as stock master

Why: brands selling on both platforms double-count or oversell; one master quantity fixes that.

- ✅ Per-brand master mode: append-only stock ledger, warehouses mapped from channel locations, ledger pushed to every channel after each change and sync, drift and oversell detection, warehouse transfers.
- ✅ Sales / wholesale orders from email or photos (`propose_sales_order`), with reservations (`available = on hand − reserved`) pushed to channels in master mode.
- ✅ Receiving POs updates master stock (ledger `receipt` entries, pushed to channels).
- ✅ Native forecasting (SES / TSB after statsforecast) with safety stock and forecast reorder points, replacing 30-day velocity.
- Weekly seasonality and promo-aware forecasts.
- Bundles / kits.

## Phase 3 — More channels & finance

- ✅ Faire (wholesale).
- Amazon, TikTok Shop, WooCommerce.
- Cost import from QuickBooks / Holded; margin and stock value at cost.
- Multi-user orgs, SSO, hosted offering.

## Non-goals (for now)

- Accounting, invoicing, full ERP.
- Built-in chat assistant — the user's own agent is the assistant.
- Warehouse picking / WMS.
