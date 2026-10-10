//! MCP server: the agent's interface to Bluestone.
use crate::auth::{Principal, Scope};
use crate::service::*;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const INSTRUCTIONS: &str = "Bluestone is the inventory hub for Joan's Shopify and PrestaShop brands. \
Use read tools to inspect stock, velocity and days of cover. Organise tools (tags, suppliers, reorder settings, notes, merges) apply immediately. \
Anything that would change a store (stock, price, listing status) must go through propose_* tools: it creates a proposal a human approves in the Bluestone UI. \
Brands in master mode keep stock in Bluestone's ledger: stock proposals and transfers target a warehouse, and approved changes are pushed to every channel; use stock_ledger to see why stock moved. \
Always include a short rationale in proposals. Items are addressed by id or SKU; pass `brand` if a SKU exists in several brands and `channel` if an item is listed on several channels.";

#[derive(Clone)]
pub struct BluestoneMcp {
    svc: Arc<Service>,
    /// Fixed identity for stdio sessions; HTTP sessions authenticate per request.
    stdio_principal: Option<Principal>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ItemArgs {
    /// Item id or SKU.
    pub item: String,
    pub brand: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LowStockArgs {
    pub brand: Option<String>,
    /// Only items with fewer days of cover than this. Defaults to each item's lead time (status low/out).
    pub max_days_cover: Option<f64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SalesArgs {
    /// Look-back window in days (default 30, max 365).
    pub days: Option<i64>,
    pub brand: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ProposalsArgs {
    /// pending, applied, failed, rejected or all (default pending).
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ActivityArgs {
    pub limit: Option<i64>,
    /// agent, human or system.
    pub actor_kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SyncArgs {
    /// Channel id; omit to sync every channel.
    pub channel_id: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TagArgs {
    /// Item ids or SKUs.
    pub items: Vec<String>,
    pub brand: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SupplierArgs {
    pub items: Vec<String>,
    pub brand: Option<String>,
    /// Supplier name (created if new). Empty string clears the supplier.
    pub supplier: String,
    pub email: Option<String>,
    /// Default lead time for this supplier.
    pub lead_time_days: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReorderArgs {
    pub item: String,
    pub brand: Option<String>,
    /// Item counts as low stock at or below this quantity.
    pub reorder_point: Option<i64>,
    /// Desired stock after a reorder.
    pub target_stock: Option<i64>,
    /// Overrides the supplier's lead time for this item.
    pub lead_time_days: Option<i64>,
    pub unit_cost: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NoteArgs {
    pub item: String,
    pub brand: Option<String>,
    pub body: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MergeArgs {
    /// Item to fold in (archived afterwards).
    pub source: String,
    /// Item that keeps all listings.
    pub target: String,
    pub brand: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LedgerArgs {
    /// Item id or SKU.
    pub item: String,
    pub brand: Option<String>,
    /// Most recent entries to return (default 50).
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WarehousesArgs {
    pub brand: Option<String>,
}

fn json_result<T: Serialize>(v: &T) -> Result<CallToolResult, McpError> {
    let text = serde_json::to_string_pretty(v)
        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn tool_error(e: ServiceError) -> Result<CallToolResult, McpError> {
    let msg = match e {
        ServiceError::Internal(e) => format!("internal error: {e:#}"),
        other => other.to_string(),
    };
    Ok(CallToolResult::error(vec![ContentBlock::text(msg)]))
}

fn done<T: Serialize>(r: SResult<T>) -> Result<CallToolResult, McpError> {
    match r {
        Ok(v) => json_result(&v),
        Err(e) => tool_error(e),
    }
}

impl BluestoneMcp {
    pub fn new(svc: Arc<Service>, stdio_principal: Option<Principal>) -> Self {
        Self {
            svc,
            stdio_principal,
            tool_router: Self::tool_router(),
        }
    }

    fn principal(&self, ctx: &RequestContext<RoleServer>) -> Result<Principal, McpError> {
        ctx.extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<Principal>().cloned())
            .or_else(|| self.stdio_principal.clone())
            .ok_or_else(|| McpError::invalid_request("unauthenticated", None))
    }

    fn reader(
        &self,
        ctx: &RequestContext<RoleServer>,
    ) -> Result<Result<Principal, ServiceError>, McpError> {
        let p = self.principal(ctx)?;
        Ok(p.require(Scope::Read).map(|_| p))
    }

    async fn resolve_many(&self, items: &[String], brand: Option<&str>) -> SResult<Vec<i64>> {
        let mut ids = Vec::new();
        for i in items {
            ids.push(self.svc.resolve_item(i, brand).await?);
        }
        Ok(ids)
    }
}

#[tool_router]
impl BluestoneMcp {
    #[tool(
        description = "Dashboard: KPI totals, brands with channel health, items needing attention, pending approvals and recent activity."
    )]
    async fn overview(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(self.svc.overview().await)
    }

    #[tool(
        description = "List brands with their Shopify/PrestaShop channels, item counts and low/out-of-stock counts."
    )]
    async fn list_brands(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(self.svc.list_brands().await)
    }

    #[tool(
        description = "Search items by text, brand, tag, supplier, status (out|low|ok|attention) or max days of cover. Returns stock, velocity, days cover and suggested reorder qty."
    )]
    async fn search_items(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(f): Parameters<ItemFilter>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(self.svc.search_items(&f).await)
    }

    #[tool(
        description = "Full detail for one item: per-channel listings and stock by location, 90-day daily sales, notes and proposal history."
    )]
    async fn get_item(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<ItemArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        let r = async {
            self.svc
                .get_item(self.svc.resolve_item(&a.item, a.brand.as_deref()).await?)
                .await
        }
        .await;
        done(r)
    }

    #[tool(
        description = "Items that are out of stock or running low (days of cover below lead time or under reorder point), most urgent first."
    )]
    async fn list_low_stock(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<LowStockArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        let f = ItemFilter {
            brand: a.brand,
            status: a.max_days_cover.is_none().then(|| "attention".into()),
            max_days_cover: a.max_days_cover,
            limit: a.limit.or(Some(50)),
            ..Default::default()
        };
        done(self.svc.search_items(&f).await)
    }

    #[tool(description = "Units, revenue and orders over a period, by day, with the top 10 items.")]
    async fn sales_summary(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<SalesArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(
            self.svc
                .sales_summary(a.days.unwrap_or(30), a.brand.as_deref())
                .await,
        )
    }

    #[tool(
        description = "List proposals (default: pending) with before/after diff, warnings and decision status."
    )]
    async fn list_proposals(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<ProposalsArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(
            self.svc
                .list_proposals(Some(a.status.as_deref().unwrap_or("pending")))
                .await,
        )
    }

    #[tool(description = "List tags with item counts.")]
    async fn list_tags(&self, ctx: RequestContext<RoleServer>) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(self.svc.tags().await)
    }

    #[tool(description = "List suppliers with contact, lead time and item counts.")]
    async fn list_suppliers(
        &self,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(self.svc.suppliers().await)
    }

    #[tool(
        description = "Recent activity log entries (syncs, organise actions, proposals, approvals)."
    )]
    async fn recent_activity(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<ActivityArgs>,
    ) -> Result<CallToolResult, McpError> {
        if let Err(e) = self.reader(&ctx)? {
            return tool_error(e);
        }
        done(
            self.svc
                .activity(a.limit.unwrap_or(30), a.actor_kind.as_deref())
                .await,
        )
    }

    #[tool(
        description = "Pull fresh products, stock and orders from Shopify/PrestaShop into Bluestone."
    )]
    async fn sync_channels(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<SyncArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = match self.reader(&ctx)? {
            Ok(p) => p,
            Err(e) => return tool_error(e),
        };
        match a.channel_id {
            Some(id) => done(self.svc.sync_channel(&p, id).await.map(|r| vec![r])),
            None => done(self.svc.sync_all(&p).await),
        }
    }

    #[tool(description = "Add tags to items (applies immediately, logged).")]
    async fn tag_items(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<TagArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            let ids = self.resolve_many(&a.items, a.brand.as_deref()).await?;
            self.svc.tag_items(&p, &ids, &a.tags, false).await?;
            Ok(serde_json::json!({ "tagged": ids.len(), "tags": a.tags }))
        }
        .await;
        done(r)
    }

    #[tool(description = "Remove tags from items (applies immediately, logged).")]
    async fn untag_items(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<TagArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            let ids = self.resolve_many(&a.items, a.brand.as_deref()).await?;
            self.svc.tag_items(&p, &ids, &a.tags, true).await?;
            Ok(serde_json::json!({ "untagged": ids.len(), "tags": a.tags }))
        }
        .await;
        done(r)
    }

    #[tool(
        description = "Assign a supplier to items, creating the supplier if new (applies immediately, logged)."
    )]
    async fn set_supplier(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<SupplierArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            let ids = self.resolve_many(&a.items, a.brand.as_deref()).await?;
            let supplier = Some(a.supplier.as_str()).filter(|s| !s.trim().is_empty());
            self.svc
                .set_supplier(&p, &ids, supplier, a.email.as_deref(), a.lead_time_days)
                .await?;
            Ok(serde_json::json!({ "updated": ids.len(), "supplier": supplier }))
        }
        .await;
        done(r)
    }

    #[tool(
        description = "Set reorder point, target stock, lead time or unit cost for an item (applies immediately, logged)."
    )]
    async fn set_reorder_settings(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<ReorderArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            let id = self.svc.resolve_item(&a.item, a.brand.as_deref()).await?;
            let s = ItemSettings {
                reorder_point: a.reorder_point,
                target_stock: a.target_stock,
                lead_time_days: a.lead_time_days,
                unit_cost: a.unit_cost,
            };
            self.svc.update_item(&p, id, &s).await?;
            Ok(self.svc.get_item(id).await?.summary)
        }
        .await;
        done(r)
    }

    #[tool(description = "Attach a note to an item (applies immediately, logged).")]
    async fn add_note(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<NoteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            let id = self.svc.resolve_item(&a.item, a.brand.as_deref()).await?;
            self.svc.add_note(&p, id, &a.body).await?;
            Ok(serde_json::json!({ "ok": true, "item_id": id }))
        }
        .await;
        done(r)
    }

    #[tool(
        description = "Merge two items of the same brand (e.g. the same product listed under different SKUs on Shopify and PrestaShop). Source listings, tags and notes move to target; source is archived."
    )]
    async fn merge_items(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<MergeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            let s = self.svc.resolve_item(&a.source, a.brand.as_deref()).await?;
            let t = self.svc.resolve_item(&a.target, a.brand.as_deref()).await?;
            self.svc.merge_items(&p, s, t).await?;
            Ok(self.svc.get_item(t).await?.summary)
        }
        .await;
        done(r)
    }

    #[tool(
        description = "Stock ledger for an item: quantity per warehouse and the latest entries (initial, sale, adjustment, transfer, receipt, correction) with who and why. Only brands in master mode have a ledger."
    )]
    async fn stock_ledger(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<LedgerArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            p.require(Scope::Read)?;
            let id = self.svc.resolve_item(&a.item, a.brand.as_deref()).await?;
            Ok(serde_json::json!({
                "stock_mode": self.svc.item_stock_mode(id).await?,
                "levels": self.svc.stock_levels(id).await?,
                "entries": self.svc.ledger(id, a.limit.unwrap_or(50)).await?,
            }))
        }
        .await;
        done(r)
    }

    #[tool(
        description = "List Bluestone warehouses (master-mode brands) with the channel locations they feed and total units."
    )]
    async fn list_warehouses(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<WarehousesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let r = async {
            p.require(Scope::Read)?;
            self.svc.warehouses(a.brand.as_deref()).await
        }
        .await;
        done(r)
    }

    #[tool(
        description = "Propose a sales / wholesale order you read from an email, message or photo of an order sheet. Include `source` (where it came from) and the customer's reference. It waits for a human to confirm; confirming reserves the stock. The response lists lines with current availability and warnings for shortfalls."
    )]
    async fn propose_sales_order(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::sales::ProposeSalesOrder>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.propose_sales_order(&p, &a).await)
    }

    #[tool(
        description = "List sales orders (default: open = proposed + confirmed) with lines, availability and shortfall warnings."
    )]
    async fn list_sales_orders(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::sales::SoListArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let status = a.status.unwrap_or_else(|| "open".into());
        done(
            self.svc
                .sales_orders(&p, Some(&status), a.brand.as_deref())
                .await,
        )
    }

    #[tool(description = "Get one sales order with its lines.")]
    async fn get_sales_order(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::sales::SoIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.sales_order(&p, a.id).await)
    }

    #[tool(
        description = "List purchase orders (default: open ones) with lines, received quantities and cost."
    )]
    async fn list_purchase_orders(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::purchasing::PoListArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        let status = a.status.unwrap_or_else(|| "open".into());
        done(
            self.svc
                .purchase_orders(&p, Some(&status), a.brand.as_deref())
                .await,
        )
    }

    #[tool(description = "Get one purchase order with its lines.")]
    async fn get_purchase_order(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::purchasing::PoIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.purchase_order(&p, a.id).await)
    }

    #[tool(
        description = "Draft a purchase order for a brand. Pass `lines` (item + quantity), or just a `supplier` to add every low/out item from that supplier at its suggested quantity (net of what is already on order). The draft waits for a human to approve and send it."
    )]
    async fn draft_purchase_order(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::purchasing::DraftPo>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.draft_po(&p, &a).await)
    }

    #[tool(
        description = "Record goods received on an approved/sent purchase order (all outstanding, or specific lines). Master brands: adds stock to the PO's warehouse and pushes it to every channel. Human tokens only."
    )]
    async fn receive_purchase_order(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::purchasing::ReceivePoArgs>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.receive_po(&p, a.id, a.lines.as_deref()).await)
    }

    #[tool(
        description = "Propose moving stock between two warehouses of a master-mode brand. Creates a pending proposal; on approval the ledger records both legs and channels are updated."
    )]
    async fn propose_stock_transfer(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<crate::ledger::ProposeTransfer>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.propose_transfer(&p, &a).await)
    }

    #[tool(
        description = "Propose setting stock for an item, as an absolute `quantity` or a `delta`. Mirror brands: targets a channel/location and is written to that store on approval. Master brands: `location` is a Bluestone warehouse, the change lands in the ledger and is pushed to every channel. Nothing changes until a human approves."
    )]
    async fn propose_stock_adjustment(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<ProposeStock>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.propose_stock(&p, &a).await)
    }

    #[tool(
        description = "Propose a new price for an item on a channel. Creates a pending proposal for human approval."
    )]
    async fn propose_price_change(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<ProposePrice>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.propose_price(&p, &a).await)
    }

    #[tool(
        description = "Propose activating, drafting or archiving an item's listing on a channel. Creates a pending proposal for human approval."
    )]
    async fn propose_listing_status(
        &self,
        ctx: RequestContext<RoleServer>,
        Parameters(a): Parameters<ProposeStatus>,
    ) -> Result<CallToolResult, McpError> {
        let p = self.principal(&ctx)?;
        done(self.svc.propose_status(&p, &a).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for BluestoneMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("bluestone", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}
