# Bluestone — spec (v0)

Bluestone is an inventory **visualisation and organisation hub** for Shopify and PrestaShop brands, built for an AI agent to work in. The agent (Claude Code, or a custom agent) connects over **MCP**; the human uses the **web UI**. Both act on the same records and write to the same activity log.

It is *not* an ERP. In v0, Shopify and PrestaShop stay the source of truth for stock, price and listing status. Bluestone mirrors them, adds the organisation layer they lack (tags, suppliers, reorder points, notes, cross-channel item grouping), computes velocity and days of cover, and pushes back only changes a human approved.

## Principles

1. **Agent-first.** Every capability ships as an MCP tool first; the UI is a view over the same service layer.
2. **Organise freely, write carefully.** Organisation tools apply instantly and are logged. Anything that changes a store is a *proposal* with a before/after diff, waiting for a human.
3. **Agents never approve.** Agent tokens can't hold the `approve` scope through MCP; approval is a UI action by a human token.
4. **No credentials in the database.** Channels reference an env var name (`credential_env`); the token lives in the environment / secret store.
5. **Everything is attributable.** Each activity row records who (agent or human, token name), what, and on which subject.

## Architecture

```
Shopify (Admin GraphQL 2025-07) ─┐                      ┌─ /mcp  streamable HTTP ◄── custom agent
                                 ├─ connectors ─► SQLite ┤  `bluestone mcp` stdio ◄── Claude Code
PrestaShop (WebService, 1.7–9) ──┘   (sync)      (hub)   └─ /api  REST ◄── web UI (TanStack Start + shadcn)
```

- **server/** — Rust: axum (HTTP), sqlx (SQLite, migrations), rmcp (official MCP SDK), reqwest (connectors). Single binary `bluestone`.
- **web/** — TanStack Start + TanStack Router/Query, shadcn/ui (Radix), Tailwind v4, Recharts.
- **dev/** — docker compose for a local PrestaShop 8.2 dev shop and a Shopify Admin API mock.

## Data model

| Table | Purpose |
|---|---|
| `brands` | A brand you manage (e.g. "Northwind Coffee"). |
| `channels` | A store of a brand: `kind` (`shopify`/`prestashop`/`faire`), `base_url`, `credential_env`, sync status. |
| `locations` | Stock locations per channel (Shopify locations; PrestaShop has one, `default`). |
| `items` | Bluestone's own unit, unique per `(brand, sku)`. Holds organisation fields: supplier, reorder point, target stock, lead time, unit cost. |
| `listings` | A product/variant on one channel, linked to an item. Mirrors title, price, status, image. |
| `listing_stock` | Quantity per listing × location (+ connector `stock_ref`). |
| `stock_snapshots` | Stock total per listing at each sync / applied change (for history charts). |
| `order_lines` | Last 90 days of order lines, linked to listings; drives velocity. |
| `tags`, `item_tags` | Free-form tags (lowercased). |
| `suppliers` | Name, email, default lead time. |
| `notes` | Free text on an item, attributed. |
| `proposals` | `stock_adjustment` / `price_change` / `listing_status`, before/after JSON, rationale, status `pending → applied/failed/rejected`. |
| `activity` | Append-only log of every sync, organise action, proposal and decision. |
| `tokens` | Hashed bearer tokens with `kind` (agent/human) and scopes. |

Items are matched across channels by SKU at sync time (same brand, same SKU → same item). Listings without a SKU get a synthetic one (`shopify-<product>-<variant>`); `merge_items` folds them together.

### Derived metrics

- `on_hand` = sum of `listing_stock` across the item's listings and locations.
- `daily_velocity` = units sold in the last 30 days / 30.
- `days_cover` = `on_hand / daily_velocity` (null if nothing sold).
- `lead_time_days` = item lead time, else supplier lead time, else 14.
- `status` = `out` if on_hand ≤ 0; `low` if on_hand ≤ reorder point or days_cover < lead time; else `ok`.
- `suggested_reorder_qty` = `target_stock − on_hand` if a target is set, else `velocity × (lead time + 30) − on_hand` for low/out items.

> Open question: brands selling the same physical stock on both Shopify and PrestaShop will have on_hand double-counted in v0. Phase 2 (Bluestone as stock master) resolves this; until then tag one channel's listing as the master or keep separate SKUs.

## Auth & scopes

Bearer tokens (`bst_…`, SHA-256 hashed at rest), created with `bluestone token create`.

| Scope | Grants |
|---|---|
| `read` | all reads, triggering a sync |
| `organise` | tags, suppliers, reorder settings, notes, merges |
| `propose` | creating proposals |
| `approve` | approving / rejecting proposals (REST/UI only) |
| `admin` | everything, plus adding channels |

Typical setup: `claude-code` and `harness` agent tokens with `read,organise,propose`; a `joan` human token with `admin` for the UI.

## MCP server

Transports: **stdio** (`bluestone mcp`, token from `BLUESTONE_TOKEN`) for Claude Code, and **streamable HTTP** at `POST /mcp` (token in `Authorization: Bearer`) for custom agents.

| Tier | Tools |
|---|---|
| Read | `overview`, `list_brands`, `search_items`, `get_item`, `list_low_stock`, `sales_summary`, `list_proposals`, `list_tags`, `list_suppliers`, `recent_activity`, `sync_channels` |
| Organise | `tag_items`, `untag_items`, `set_supplier`, `set_reorder_settings`, `add_note`, `merge_items` |
| Propose | `propose_stock_adjustment`, `propose_price_change`, `propose_listing_status` |

Items are addressed by id or SKU (+ `brand` when a SKU exists in several brands). Errors are returned as tool errors with an actionable message (e.g. "item is listed on several channels (shopify-eu, presta); pass `channel`").

## REST API (for the UI)

All under `/api`, bearer auth. `GET /me`, `GET /overview`, `GET /brands`, `GET /items?query&brand&tag&supplier&status`, `GET /items/:id`, `PATCH /items/:id`, `POST /items/:id/tags`, `DELETE /items/:id/tags/:tag`, `POST /items/:id/notes`, `POST /items/supplier`, `GET /tags`, `GET /suppliers`, `GET /sales?days&brand`, `GET /proposals?status`, `POST /proposals` (`kind` + fields), `POST /proposals/:id/approve`, `POST /proposals/:id/reject`, `GET /activity?actor_kind`, `GET|POST /channels`, `POST /channels/:id/sync`, `POST /sync`.

## Connectors

**Shopify** — Admin GraphQL `2025-07`, header `X-Shopify-Access-Token` (custom-app token). Reads `locations`, `products → variants → inventoryItem.inventoryLevels(available)`, `orders(created_at ≥ 90d) → lineItems`. Writes: `inventorySetQuantities` (with `compareQuantity` = the proposal's *before*, so a stale proposal fails instead of clobbering), `productVariantsBulkUpdate` (price), `productUpdate` (status). Required scopes: `read_products, write_products, read_inventory, write_inventory, read_locations, read_orders`.

**Faire** — External API v2 (`/external-api/v2`), OAuth headers `X-FAIRE-APP-CREDENTIALS` + `X-FAIRE-OAUTH-ACCESS-TOKEN`; the channel's env var holds `<base64 app credentials>:<access token>`. Reads `products` (cursor-paged; deleted products/variants skipped), on-hand stock via `product-inventory/by-product-variant-ids` (untracked variants have no stock row), `orders(created_at_min, excluding CANCELED)` and their non-canceled items. One synthetic location, `Faire stock`. Bluestone's price for a Faire listing is the **wholesale** price. Writes: on-hand inventory (no native compare-and-set, so the current value is re-read first and a moved value fails the proposal), wholesale price in the variant's primary currency (retail and other currencies untouched), product `lifecycle_state` (`active` → `PUBLISHED`, `draft`/`archived` → `UNPUBLISHED`; never deleted). Local mock: `server/target/debug/mock_faire` (port 8789).

**PrestaShop** — legacy WebService (`/api`, HTTP basic auth with the key), JSON output. Reads `products`, `combinations`, `stock_availables`, `orders`, `order_details`. Writes GET the resource XML, strip read-only fields, patch and PUT (`stock_availables.quantity`, `products.price` / `combinations.price` impact, `products.active`). Key needs GET/PUT on those resources.

## UI (Convexity-inspired)

Light, calm operations UI: white cards on a soft grey canvas, one strong blue accent (`#2170e4`), left nav rail, dense but airy tables, status pills, small trend charts. Agent activity is first-class: the inbox of proposals looks like Convexity's "recommended actions", each with the agent's rationale and an Approve button.

| View | Content |
|---|---|
| **Overview** | KPI cards (items, low, out, pending approvals, units & revenue 30d), "Needs attention" table, sales chart, brands & channel health, recent agent activity. |
| **Inventory** | Filterable table (brand, status, tag, supplier, search): SKU, name, channels, on hand, velocity, days cover, reorder point, status, tags. Bulk tag / set supplier. |
| **Item** | Header with status, KPIs, per-channel listings with stock by location, sales chart, organisation form (supplier, reorder point, target, lead time, cost), tags, notes, proposal history, quick "propose" actions. |
| **Approvals** | Pending proposals as cards: diff, warnings, rationale, actor; Approve / Reject. Tabs for applied / failed / rejected. |
| **Reorder** | Low/out items grouped by supplier with suggested quantities (read-only in v0; feeds PO drafting in phase 1). |
| **Activity** | Timeline filterable by agent/human/system. |
| **Channels** | Brands, connected channels, last sync, errors, "Sync now"; how to connect an agent (MCP config snippets). |
