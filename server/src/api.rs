//! REST API used by the web UI.
use crate::auth::{self, Principal, Scope};
use crate::service::*;
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub type AppState = Arc<Service>;

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let code = match &self {
            ServiceError::NotFound(_) => StatusCode::NOT_FOUND,
            ServiceError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ServiceError::Forbidden(_) => StatusCode::FORBIDDEN,
            ServiceError::Conflict(_) => StatusCode::CONFLICT,
            ServiceError::Internal(e) => {
                tracing::error!(error = %format!("{e:#}"), "internal error");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let msg = match &self {
            ServiceError::Internal(e) => format!("{e:#}"),
            other => other.to_string(),
        };
        (code, Json(json!({ "error": msg }))).into_response()
    }
}

type R<T> = Result<Json<T>, ServiceError>;

/// Resolves the bearer token into a [`Principal`] request extension, or 401s.
pub async fn require_token(State(svc): State<AppState>, mut req: Request, next: Next) -> Response {
    let principal = match auth::bearer(req.headers()) {
        Some(t) => auth::authenticate(&svc.pool, t).await,
        None => None,
    };
    match principal {
        Some(p) => {
            req.extensions_mut().insert(p);
            next.run(req).await
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "missing or invalid bearer token" })),
        )
            .into_response(),
    }
}

pub fn router(svc: AppState) -> Router {
    Router::new()
        .route("/me", get(me))
        .route("/overview", get(overview))
        .route("/brands", get(brands))
        .route("/brands/{id}/stock-mode", post(stock_mode))
        .route("/warehouses", get(warehouses))
        .route("/items/{id}/ledger", get(item_ledger))
        .route("/items", get(items))
        .route("/items/{id}", get(item).patch(update_item))
        .route("/items/{id}/tags", post(add_tags))
        .route("/items/{id}/tags/{tag}", delete(remove_tag))
        .route("/items/{id}/notes", post(add_note))
        .route("/items/bulk/tags", post(bulk_tags))
        .route("/items/bulk/supplier", post(bulk_supplier))
        .route("/items/{id}/merge", post(merge))
        .route("/tags", get(tags))
        .route("/suppliers", get(suppliers))
        .route("/sales", get(sales))
        .route("/proposals", get(proposals).post(create_proposal))
        .route("/proposals/{id}", get(proposal))
        .route("/proposals/{id}/approve", post(approve))
        .route("/proposals/{id}/reject", post(reject))
        .route("/activity", get(activity))
        .route("/channels", get(channels).post(add_channel))
        .route("/channels/{id}/sync", post(sync_channel))
        .route("/sync", post(sync_all))
        .layer(axum::middleware::from_fn_with_state(
            svc.clone(),
            require_token,
        ))
        .with_state(svc)
}

async fn me(Extension(p): Extension<Principal>) -> Json<Value> {
    Json(
        json!({ "name": p.name, "kind": p.kind, "scopes": p.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>() }),
    )
}

async fn overview(State(s): State<AppState>, Extension(p): Extension<Principal>) -> R<Overview> {
    p.require(Scope::Read)?;
    Ok(Json(s.overview().await?))
}

async fn brands(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
) -> R<Vec<BrandSummary>> {
    p.require(Scope::Read)?;
    Ok(Json(s.list_brands().await?))
}

async fn items(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Query(f): Query<ItemFilter>,
) -> R<Vec<ItemSummary>> {
    p.require(Scope::Read)?;
    Ok(Json(s.search_items(&f).await?))
}

async fn item(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
) -> R<ItemDetail> {
    p.require(Scope::Read)?;
    Ok(Json(s.get_item(id).await?))
}

async fn update_item(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
    Json(b): Json<ItemSettings>,
) -> R<ItemDetail> {
    s.update_item(&p, id, &b).await?;
    Ok(Json(s.get_item(id).await?))
}

#[derive(Deserialize)]
struct TagsBody {
    tags: Vec<String>,
}

async fn add_tags(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
    Json(b): Json<TagsBody>,
) -> R<ItemDetail> {
    s.tag_items(&p, &[id], &b.tags, false).await?;
    Ok(Json(s.get_item(id).await?))
}

async fn remove_tag(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path((id, tag)): Path<(i64, String)>,
) -> R<ItemDetail> {
    s.tag_items(&p, &[id], &[tag], true).await?;
    Ok(Json(s.get_item(id).await?))
}

#[derive(Deserialize)]
struct NoteBody {
    body: String,
}

async fn add_note(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
    Json(b): Json<NoteBody>,
) -> R<ItemDetail> {
    s.add_note(&p, id, &b.body).await?;
    Ok(Json(s.get_item(id).await?))
}

#[derive(Deserialize)]
struct BulkTags {
    item_ids: Vec<i64>,
    tags: Vec<String>,
    #[serde(default)]
    remove: bool,
}

async fn bulk_tags(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Json(b): Json<BulkTags>,
) -> R<Value> {
    s.tag_items(&p, &b.item_ids, &b.tags, b.remove).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct BulkSupplier {
    item_ids: Vec<i64>,
    supplier: Option<String>,
    email: Option<String>,
    lead_time_days: Option<i64>,
}

async fn bulk_supplier(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Json(b): Json<BulkSupplier>,
) -> R<Value> {
    s.set_supplier(
        &p,
        &b.item_ids,
        b.supplier.as_deref(),
        b.email.as_deref(),
        b.lead_time_days,
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct MergeBody {
    into: i64,
}

async fn merge(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
    Json(b): Json<MergeBody>,
) -> R<ItemDetail> {
    s.merge_items(&p, id, b.into).await?;
    Ok(Json(s.get_item(b.into).await?))
}

async fn tags(State(s): State<AppState>, Extension(p): Extension<Principal>) -> R<Vec<TagCount>> {
    p.require(Scope::Read)?;
    Ok(Json(s.tags().await?))
}

async fn suppliers(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
) -> R<Vec<Supplier>> {
    p.require(Scope::Read)?;
    Ok(Json(s.suppliers().await?))
}

#[derive(Deserialize)]
struct SalesQuery {
    days: Option<i64>,
    brand: Option<String>,
}

async fn sales(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Query(q): Query<SalesQuery>,
) -> R<SalesSummary> {
    p.require(Scope::Read)?;
    Ok(Json(
        s.sales_summary(
            q.days.unwrap_or(30),
            q.brand.as_deref().filter(|b| !b.is_empty()),
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct StatusQuery {
    status: Option<String>,
}

async fn proposals(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Query(q): Query<StatusQuery>,
) -> R<Vec<ProposalView>> {
    p.require(Scope::Read)?;
    Ok(Json(s.list_proposals(q.status.as_deref()).await?))
}

async fn proposal(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
) -> R<ProposalView> {
    p.require(Scope::Read)?;
    Ok(Json(s.get_proposal(id).await?))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum NewProposal {
    StockAdjustment(ProposeStock),
    PriceChange(ProposePrice),
    ListingStatus(ProposeStatus),
    StockTransfer(crate::ledger::ProposeTransfer),
}

async fn create_proposal(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Json(b): Json<NewProposal>,
) -> R<ProposalView> {
    Ok(Json(match b {
        NewProposal::StockAdjustment(x) => s.propose_stock(&p, &x).await?,
        NewProposal::PriceChange(x) => s.propose_price(&p, &x).await?,
        NewProposal::ListingStatus(x) => s.propose_status(&p, &x).await?,
        NewProposal::StockTransfer(x) => s.propose_transfer(&p, &x).await?,
    }))
}

async fn approve(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
) -> R<ProposalView> {
    if p.kind != "human" && p.kind != "system" {
        return Err(ServiceError::Forbidden(
            "only human tokens can approve proposals".into(),
        ));
    }
    Ok(Json(s.approve(&p, id).await?))
}

#[derive(Deserialize, Default)]
struct RejectBody {
    reason: Option<String>,
}

async fn reject(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
    body: Option<Json<RejectBody>>,
) -> R<ProposalView> {
    if p.kind != "human" && p.kind != "system" {
        return Err(ServiceError::Forbidden(
            "only human tokens can reject proposals".into(),
        ));
    }
    let reason = body
        .and_then(|Json(b)| b.reason)
        .filter(|r| !r.trim().is_empty());
    Ok(Json(s.reject(&p, id, reason.as_deref()).await?))
}

#[derive(Deserialize)]
struct ActivityQuery {
    limit: Option<i64>,
    actor_kind: Option<String>,
}

async fn activity(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Query(q): Query<ActivityQuery>,
) -> R<Vec<ActivityEntry>> {
    p.require(Scope::Read)?;
    Ok(Json(
        s.activity(
            q.limit.unwrap_or(100),
            q.actor_kind
                .as_deref()
                .filter(|k| !k.is_empty() && *k != "all"),
        )
        .await?,
    ))
}

async fn channels(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
) -> R<Vec<BrandSummary>> {
    p.require(Scope::Read)?;
    Ok(Json(s.list_brands().await?))
}

async fn add_channel(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Json(b): Json<NewChannel>,
) -> R<Value> {
    let id = s.add_channel(&p, &b).await?;
    Ok(Json(json!({ "id": id })))
}

async fn sync_channel(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
) -> R<SyncReport> {
    Ok(Json(s.sync_channel(&p, id).await?))
}

async fn sync_all(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
) -> R<Vec<SyncReport>> {
    p.require(Scope::Read)?;
    Ok(Json(s.sync_all(&p).await?))
}

#[derive(Deserialize)]
struct WarehouseQuery {
    brand: Option<String>,
}

async fn warehouses(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Query(q): Query<WarehouseQuery>,
) -> R<Vec<crate::ledger::Warehouse>> {
    p.require(Scope::Read)?;
    Ok(Json(s.warehouses(q.brand.as_deref()).await?))
}

#[derive(Deserialize)]
struct LedgerQuery {
    limit: Option<i64>,
}

async fn item_ledger(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<i64>,
    Query(q): Query<LedgerQuery>,
) -> R<Value> {
    p.require(Scope::Read)?;
    Ok(Json(json!({
        "stock_mode": s.item_stock_mode(id).await?,
        "levels": s.stock_levels(id).await?,
        "entries": s.ledger(id, q.limit.unwrap_or(100)).await?,
    })))
}

#[derive(Deserialize)]
struct StockModeBody {
    mode: String,
}

async fn stock_mode(
    State(s): State<AppState>,
    Extension(p): Extension<Principal>,
    Path(id): Path<String>,
    Json(b): Json<StockModeBody>,
) -> R<Value> {
    let reconcile = s.set_stock_mode(&p, &id, &b.mode).await?;
    Ok(Json(json!({ "mode": b.mode, "reconcile": reconcile })))
}
