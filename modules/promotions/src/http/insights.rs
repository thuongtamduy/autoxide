//! Chạy thử, xung đột, lịch, số liệu (D-222). Chỉ đọc.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::Cart;
use crate::http::dto::PromotionListItem;
use crate::insights::{Conflict, DailyStats, Simulation, check_range};
use crate::model::{Kind, Status};
use crate::permissions::READ;
use crate::programs::PromotionFilter;
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(simulate))
        .routes(routes!(conflicts))
        .routes(routes!(calendar))
        .routes(routes!(stats))
        .with_state(service)
}

/// Carts to compute with and without the promotion.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsSimulateRequest)]
pub(crate) struct SimulateRequest {
    /// 1 to 20 carts, each as the body of `POST /api/v1/promotions/evaluate`.
    pub carts: Vec<Cart>,
}

// Service kiểm giỏ và số giỏ (gom lỗi một lần, D-083).
impl Validate for SimulateRequest {
    fn validate(&self) -> Result<(), validator::ValidationErrors> {
        Ok(())
    }
}

/// Try a promotion on sample carts.
///
/// Requires `promotions.read`. Each cart is computed with the promotion (even while it is a
/// draft or paused) and without it. Nothing is written or held.
#[utoipa::path(
    post,
    path = "/promotions/{id}/simulate",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "simulate_promotion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    request_body = SimulateRequest,
    responses(
        (status = 200, description = "One result per cart, in the order sent.", body = Data<Vec<Simulation>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 422, description = "No cart or more than 20, or a cart is invalid; see `errors`.", body = Problem),
    )
)]
async fn simulate(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<SimulateRequest>,
) -> Result<Data<Vec<Simulation>>, Error> {
    principal.require(&READ)?;
    Ok(Data { data: service.simulate(&tenant, &principal, id, &body.carts).await? })
}

/// List the promotions that may clash with this one.
///
/// Requires `promotions.read`. Other promotions of the same unit, at the same time, on shared
/// targets and in the same class, where one side is exclusive: only one of them applies to a
/// line. A warning, never a block (`activate` returns it in `warnings`).
#[utoipa::path(
    get,
    path = "/promotions/{id}/conflicts",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "list_promotion_conflicts",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion.")),
    responses(
        (status = 200, description = "The clashing promotions (empty when none).", body = Data<Vec<Conflict>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
    )
)]
async fn conflicts(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<Conflict>>, Error> {
    principal.require(&READ)?;
    Ok(Data { data: service.conflicts(&tenant, &principal, id).await? })
}

/// A range of days (Vietnam dates, both included).
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct RangeQuery {
    /// First day, `YYYY-MM-DD`.
    pub from: NaiveDate,
    /// Last day, `YYYY-MM-DD`; at most 366 days after `from`.
    pub to: NaiveDate,
}

/// Days and unit of the calendar.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CalendarQuery {
    /// First day, `YYYY-MM-DD` (Vietnam date).
    pub from: NaiveDate,
    /// Last day, `YYYY-MM-DD`; at most 366 days after `from`.
    pub to: NaiveDate,
    /// Only promotions of this unit.
    pub org_id: Option<Uuid>,
}

/// List the promotions running on some days (calendar).
///
/// Requires `promotions.read`. Promotions in your scope whose time overlaps the days, scheduled,
/// running, paused or ended (not drafts or cancelled ones). Oldest first; follow
/// `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/promotions/calendar",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "promotion_calendar",
    security(("bearer" = [])),
    params(CalendarQuery, PageQuery),
    responses(
        (status = 200, description = "One page of promotions.", body = Page<PromotionListItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`from` is after `to` or the range is longer than 366 days; see `errors`.", body = Problem),
    )
)]
async fn calendar(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CalendarQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<PromotionListItem>, Error> {
    principal.require(&READ)?;
    let page = PageRequest::try_from(page)?;
    check_range(query.from, query.to)?;
    let filter = PromotionFilter {
        org_id: query.org_id,
        window: Some((
            crate::compliance::day_start(query.from),
            crate::compliance::day_start(query.to + chrono::Days::new(1)),
        )),
        ..PromotionFilter::default()
    };
    let found = service.list_promotions(&tenant, &principal, &filter, &page).await?;
    Ok(found.map(|(row, title)| PromotionListItem {
        id: row.id,
        org_id: row.org_id,
        title,
        kind: Kind::parse(&row.kind).unwrap_or(Kind::PercentOff),
        status: Status::parse(&row.status).unwrap_or(Status::Draft),
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        usage_count: row.usage_count,
    }))
}

/// Get the daily counts of a promotion.
///
/// Requires `promotions.read`. Confirmed uses, discount given, order value and who paid, per
/// Vietnam date; days without use are left out. Counts only, no buyer data.
#[utoipa::path(
    get,
    path = "/promotions/{id}/stats",
    extensions(("x-permissions" = json!(["promotions.read"]))),
    operation_id = "get_promotion_stats",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion."), RangeQuery),
    responses(
        (status = 200, description = "The counts, oldest day first.", body = Data<Vec<DailyStats>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
        (status = 422, description = "`from` is after `to` or the range is longer than 366 days; see `errors`.", body = Problem),
    )
)]
async fn stats(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(range): Query<RangeQuery>,
) -> Result<Data<Vec<DailyStats>>, Error> {
    principal.require(&READ)?;
    Ok(Data { data: service.stats(&tenant, &principal, id, range.from, range.to).await? })
}
