//! Endpoint dòng tồn, sổ biến động và điều chỉnh tồn, dưới `/api/v1/inventory`.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::http::dto::{
    AdjustmentRequest, AdjustmentResponse, KindDto, LevelResponse, MovementResponse,
};
use crate::levels::{AdjustInput, LevelFilter, MovementFilter};
use crate::permissions::{ADJUST, READ, SETTINGS_MANAGE};
use crate::service::InventoryService;

pub(crate) fn routes(service: Arc<InventoryService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_levels))
        .routes(routes!(get_level, update_level))
        .routes(routes!(list_movements))
        .routes(routes!(create_adjustment))
        .routes(routes!(get_adjustment))
        .routes(routes!(list_lots))
        .routes(routes!(valuation))
        .with_state(service)
}

/// Filters of the stock level list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct LevelsQuery {
    /// Only this warehouse or store.
    pub org_id: Option<Uuid>,
    /// Only this variant.
    pub variant_id: Option<Uuid>,
    /// `true`: only levels whose available stock is at or below their reorder point.
    pub below_reorder: Option<bool>,
}

impl From<LevelsQuery> for LevelFilter {
    fn from(query: LevelsQuery) -> Self {
        LevelFilter {
            org_id: query.org_id,
            variant_id: query.variant_id,
            below_reorder: query.below_reorder.unwrap_or(false),
        }
    }
}

/// List stock levels (variant × warehouse) in your scope.
///
/// Requires `inventory.read`; its scope decides which warehouses you see.
#[utoipa::path(
    get,
    path = "/levels",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "list_inventory_levels",
    security(("bearer" = [])),
    params(LevelsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of stock levels.", body = Page<LevelResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_levels(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<LevelsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<LevelResponse>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let found = service.list_levels(&tenant, &principal, &query.into(), &page).await?;
    Ok(found.map(LevelResponse::from))
}

/// Get one stock level.
///
/// Requires `inventory.read` covering the warehouse; a level outside your scope answers 404.
#[utoipa::path(
    get,
    path = "/levels/{id}",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "get_inventory_level",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the stock level.")),
    responses(
        (status = 200, description = "The stock level.", body = Data<LevelResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No stock level with this id in your scope; code `inventory.level_not_found`.", body = Problem),
    )
)]
async fn get_level(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<LevelResponse>, Error> {
    principal.require(&READ)?;
    let row = service.get_level(&tenant, &principal, id).await?;
    Ok(Data { data: row.into() })
}

/// Settings of a stock level.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = InventoryLevelUpdate)]
pub(crate) struct LevelUpdateRequest {
    /// Reorder point, 0 to 1 000 000; `null` turns low-stock alerts off.
    pub reorder_point: Option<i64>,
}

/// Change the reorder point of a stock level.
///
/// Requires `inventory.settings.manage` covering the warehouse. `inventory.low_stock` fires
/// when the available stock falls from above the reorder point to it or below.
#[utoipa::path(
    patch,
    path = "/levels/{id}",
    extensions(("x-permissions" = json!(["inventory.settings.manage"]))),
    operation_id = "update_inventory_level",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the stock level.")),
    request_body = LevelUpdateRequest,
    responses(
        (status = 200, description = "The stock level.", body = Data<LevelResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.settings.manage` (`permission_denied`) or the unit is frozen (`org_frozen`).", body = Problem),
        (status = 404, description = "No stock level with this id in your scope; code `inventory.level_not_found`.", body = Problem),
        (status = 422, description = "The reorder point is out of range; see `errors`.", body = Problem),
    )
)]
async fn update_level(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<LevelUpdateRequest>,
) -> Result<Data<LevelResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let row = service.set_reorder_point(&tenant, &principal, id, input.reorder_point).await?;
    Ok(Data { data: row.into() })
}

/// Filters of the movement ledger.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct MovementsQuery {
    /// Only this warehouse or store.
    pub org_id: Option<Uuid>,
    /// Only this variant.
    pub variant_id: Option<Uuid>,
    /// Only this kind of line.
    #[param(inline)]
    pub kind: Option<KindDto>,
    /// Lines written at or after this time.
    pub from: Option<Timestamp>,
    /// Lines written before this time.
    pub to: Option<Timestamp>,
}

/// List the stock movement ledger in your scope.
///
/// Requires `inventory.read`. Newest first unless `order=asc`. The ledger is append-only: each
/// receipt, sale, return, adjustment, count, transfer or expiry adds lines and never changes
/// them.
#[utoipa::path(
    get,
    path = "/movements",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "list_inventory_movements",
    security(("bearer" = [])),
    params(MovementsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of ledger lines.", body = Page<MovementResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_movements(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<MovementsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<MovementResponse>, Error> {
    principal.require(&READ)?;
    let filter = MovementFilter {
        org_id: query.org_id,
        variant_id: query.variant_id,
        kind: query.kind.map(Into::into),
        from: query.from,
        to: query.to,
    };
    let page = PageRequest::try_from(page)?;
    let found = service.list_movements(&tenant, &principal, &filter, &page).await?;
    Ok(found.map(MovementResponse::from))
}

/// Adjust stock in one warehouse, each line with a reason.
///
/// Requires `inventory.adjust` covering the warehouse. Every line needs a `reason_code` from the
/// tenant's reason list `inventory.adjustment`. On hand never goes below 0. Send the same
/// `reference` and content again to get the adjustment already made (`200`).
#[utoipa::path(
    post,
    path = "/adjustments",
    extensions(("x-permissions" = json!(["inventory.adjust"]))),
    operation_id = "create_inventory_adjustment",
    security(("bearer" = [])),
    request_body = AdjustmentRequest,
    responses(
        (status = 200, description = "The adjustment was made before with this `reference` and content.", body = Data<AdjustmentResponse>),
        (status = 201, description = "The adjustment.", body = Data<AdjustmentResponse>, headers(("Location" = String, description = "Path of the adjustment."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.adjust` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`), or the plan does not allow another warehouse (`feature_not_enabled`, `limit_exceeded`).", body = Problem),
        (status = 404, description = "A variant does not exist; code `inventory.variant_not_found`.", body = Problem),
        (status = 409, description = "On hand would go below 0; code `inventory.insufficient_stock`.", body = Problem),
        (status = 422, description = "A line has no reason (`inventory.reason_required`) or an unknown one (`kernel.reason_unknown`), the unit holds no stock (`inventory.not_a_warehouse`), the variant tracks no stock (`inventory.not_tracked`), the cost currency differs (`inventory.currency_mismatch`), the reference was used for other content (`reference_reused`), or a value is invalid.", body = Problem),
    )
)]
async fn create_adjustment(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<AdjustmentRequest>,
) -> Result<Response, Error> {
    principal.require(&ADJUST)?;
    let input = AdjustInput {
        org_id: input.org_id,
        reference: input.reference,
        lines: input.lines.into_iter().map(Into::into).collect(),
    };
    Ok(match service.adjust(&tenant, &principal, input).await? {
        Submitted::Created(adjustment) => {
            let response = AdjustmentResponse::from(adjustment);
            Created::new(format!("/inventory/adjustments/{}", response.id), response)
                .into_response()
        }
        Submitted::Replayed(adjustment) => {
            Data { data: AdjustmentResponse::from(adjustment) }.into_response()
        }
    })
}

/// Get a stock adjustment with the ledger lines it wrote.
///
/// Requires `inventory.read` covering the warehouse; an adjustment outside your scope answers
/// 404.
#[utoipa::path(
    get,
    path = "/adjustments/{id}",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "get_inventory_adjustment",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the adjustment.")),
    responses(
        (status = 200, description = "The adjustment.", body = Data<AdjustmentResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No adjustment with this id in your scope; code `inventory.adjustment_not_found`.", body = Problem),
    )
)]
async fn get_adjustment(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<AdjustmentResponse>, Error> {
    principal.require(&READ)?;
    let adjustment = service.get_adjustment(&tenant, &principal, id).await?;
    Ok(Data { data: adjustment.into() })
}

/// Filters of the lot list.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct LotsQuery {
    /// Only lots in this warehouse or store; its setting `inventory.expiring_days` decides
    /// `expiring` (else the tenant's root setting, default 30 days).
    pub org_id: Option<Uuid>,
    /// Only lots of this variant.
    pub variant_id: Option<Uuid>,
    /// `ok`, `expiring` or `expired`.
    pub expiry_state: Option<String>,
}

/// A lot of a variant in a warehouse.
#[derive(Debug, Clone, serde::Serialize, ToSchema)]
#[schema(as = InventoryLot)]
pub(crate) struct LotResponse {
    pub id: Uuid,
    pub level_id: Uuid,
    pub org_id: Uuid,
    pub variant_id: Uuid,
    pub lot_code: String,
    pub expires_on: Option<Date>,
    /// Units of this lot on hand.
    pub on_hand: i64,
    /// `ok`, `expiring` (within `inventory.expiring_days`) or `expired`, computed when read.
    pub expiry_state: String,
    pub received_at: Timestamp,
}

impl From<crate::lots::LotView> for LotResponse {
    fn from(view: crate::lots::LotView) -> Self {
        let row = view.row;
        LotResponse {
            id: row.id,
            level_id: row.level_id,
            org_id: row.org_id,
            variant_id: row.variant_id,
            lot_code: row.lot_code,
            expires_on: row.expires_on,
            on_hand: row.on_hand,
            expiry_state: view.expiry_state.to_owned(),
            received_at: row.received_at,
        }
    }
}

/// List lots (with expiry dates) in your scope.
///
/// Requires `inventory.read`. Lots that expired are written off by a daily job (ledger lines
/// `expiry`); sales take the lots that expire first unless a lot is named.
#[utoipa::path(
    get,
    path = "/lots",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "list_inventory_lots",
    security(("bearer" = [])),
    params(LotsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of lots.", body = Page<LotResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_lots(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<LotsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<LotResponse>, Error> {
    principal.require(&READ)?;
    let filter = crate::lots::LotFilter {
        org_id: query.org_id,
        variant_id: query.variant_id,
        expiry_state: query.expiry_state,
    };
    let page = PageRequest::try_from(page)?;
    let found = service.list_lots(&tenant, &principal, &filter, &page).await?;
    Ok(found.map(LotResponse::from))
}

/// Filter of the stock valuation.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ValuationQuery {
    /// Only this warehouse or store.
    pub org_id: Option<Uuid>,
}

/// Value of the stock of one warehouse in one currency.
#[derive(Debug, Clone, serde::Serialize, ToSchema)]
#[schema(as = InventoryValuation)]
pub(crate) struct ValuationResponse {
    pub org_id: Uuid,
    /// Units on hand that have an average cost.
    pub units: i64,
    /// Sum of on hand × weighted average unit cost.
    pub value: autoxide_kernel::money::Money,
}

/// Value of the stock now: on hand × weighted average cost, per warehouse and currency.
///
/// Requires `inventory.read`; only warehouses in your scope. Units without a known cost are
/// left out. By warehouse id then currency unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/reports/valuation",
    extensions(("x-permissions" = json!(["inventory.read"]))),
    operation_id = "inventory_valuation",
    security(("bearer" = [])),
    params(ValuationQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the value per warehouse and currency.", body = Page<ValuationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `inventory.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn valuation(
    State(service): State<Arc<InventoryService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ValuationQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<ValuationResponse>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = service.valuation(&tenant, &principal, query.org_id, &page).await?;
    let mut data = Vec::with_capacity(rows.data.len());
    for row in rows.data {
        data.push(ValuationResponse {
            org_id: row.org_id,
            units: row.units,
            value: autoxide_kernel::money::Money::from_columns(row.value_minor, &row.currency)?,
        });
    }
    Ok(Page { data, pagination: rows.pagination })
}
