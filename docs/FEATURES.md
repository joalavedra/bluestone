# Bluestone — features

Mapped against Convexity (theconvexity.com) so it's clear what we copy, what we change and what we skip.
Status: ✅ in v0 · 🟡 partial · 🔜 planned (see ROADMAP.md) · ✖ out of scope.

## Inventory visibility

| Feature | Convexity | Bluestone | Status |
|---|---|---|---|
| Unified catalog across channels | ✔ | Items joined across Shopify + PrestaShop by SKU, per brand | ✅ |
| Stock by location | ✔ | Shopify locations; PrestaShop shop stock | ✅ |
| Sales velocity & days of cover | ✔ | 30-day velocity, days cover, status (out / low / ok) | ✅ |
| Low-stock / needs-attention list | ✔ | Overview + Inventory filter + `list_low_stock` tool | ✅ |
| Stock history | ✔ | Snapshots on every sync and applied change | 🟡 stored, chart in phase 1 |
| Demand forecast | ✔ (ML) | statsforecast sidecar (AutoETS / Croston for slow movers) | 🔜 phase 2 |
| Multi-brand view | partial | First-class: brands → channels → items | ✅ |

## Organisation (the layer stores lack)

| Feature | Convexity | Bluestone | Status |
|---|---|---|---|
| Tags | – | Free-form tags, bulk tag in UI, `tag_items` | ✅ |
| Suppliers & lead times | ✔ | Supplier per item, lead time per supplier or item | ✅ |
| Reorder point / target stock | ✔ | Per item; drives status + suggested qty | ✅ |
| Notes | – | Attributed notes on items (agent or human) | ✅ |
| Merge duplicate items | – | `merge_items` across mismatched SKUs | ✅ |
| Collections / saved views | – | Saved filters shared between UI and agent | 🔜 phase 1 |

## Agent interface (MCP)

| Feature | Convexity | Bluestone | Status |
|---|---|---|---|
| MCP server | ✔ (Claude, ChatGPT, Gemini) | Primary interface; stdio for Claude Code, streamable HTTP for custom agents | ✅ |
| Read tools | ✔ | overview, search, item detail, low stock, sales, proposals, tags, suppliers, activity, sync | ✅ |
| Organise tools | – | tag, supplier, reorder settings, notes, merge — applied instantly, logged | ✅ |
| Staged writes ("approve bulk changes") | ✔ | Every store write is a proposal with diff + policy warnings | ✅ |
| Per-agent tokens & scopes | – | `read / organise / propose / approve / admin`, hashed tokens | ✅ |
| Agents can't approve | – | Approval is human-only (REST/UI) | ✅ |
| Policy rules (auto-approve under limits) | – | Valet-style rules: e.g. auto-apply stock corrections < 5 units | 🔜 phase 1 |
| Order photo → sales order | ✔ | Agent reads the email/photo and calls `propose_sales_order`; human confirms (reserves stock) and fulfils (ledger sale) | ✅ |

## Writes to stores

| Feature | Convexity | Bluestone | Status |
|---|---|---|---|
| Stock adjustments | ✔ | Shopify `inventorySetQuantities` (compare-and-set), PrestaShop `stock_availables` | ✅ |
| Price changes | ✔ | Shopify variant price, PrestaShop product/combination price | ✅ |
| Listing status | – | active / draft / archived | ✅ |
| Purchase orders | ✔ | Draft per supplier from Reorder or by the agent (`draft_purchase_order`), approve → sent → partial/full receipt; receipts post to the ledger in master mode; on-order netted from suggestions. Email/PDF export pending | ✅ |
| Cross-channel stock sync | ✔ | Per-brand master mode: stock ledger + warehouses, pushes quantity to every channel, drift & oversell detection | ✅ |
| Stock transfers | ✔ | Between warehouses, proposed by the agent, recorded as two ledger legs | ✅ |

## Human UI

| View | Convexity | Bluestone | Status |
|---|---|---|---|
| Overview dashboard | ✔ | KPIs, needs attention, sales chart, channel health, agent activity | ✅ |
| Inventory table | ✔ | Filters, bulk organise | ✅ |
| Item page | ✔ | Listings × locations, sales chart, organise form, notes, proposals | ✅ |
| Recommended actions / approvals | ✔ | Approvals inbox with diff, warnings, rationale | ✅ |
| Reorder list by supplier | ✔ | Grouped low/out items + suggested qty | ✅ (read-only) |
| Activity log | – | Agent / human / system timeline | ✅ |
| Channels & agent setup | onboarding call | Sync status + copy-paste MCP config | ✅ |
| In-app chat assistant | ✔ | ✖ — your own agent is the assistant | ✖ |

## Integrations

| Integration | Convexity | Bluestone | Status |
|---|---|---|---|
| Shopify | ✔ | Admin GraphQL 2025-07, custom-app token | ✅ |
| PrestaShop | – | Legacy WebService (1.7–9) | ✅ |
| Faire (wholesale) | ✔ | External API v2: products/variants, on-hand inventory, wholesale price, orders | ✅ |
| Amazon, TikTok Shop | ✔ | – | 🔜 phase 3 |
| QuickBooks / Holded (costs) | ✔ | Unit cost import | 🔜 phase 3 |
| Webhooks (real-time) | ✔ | Shopify `inventory_levels/update`, `orders/create` | 🔜 phase 1 |
