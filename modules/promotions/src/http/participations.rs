//! Người bán tham gia chương trình của sàn (D-222).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::participations::{ParticipationRequest, ParticipationView};
use crate::permissions::{PARTICIPATE, PUBLISH, READ};
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_participations, participate))
        .routes(routes!(approve_participation))
        .routes(routes!(reject_participation))
        .with_state(service)
}

// Service kiểm mọi trường và gom lỗi một lần (D-083).
impl Validate for ParticipationRequest {
    fn validate(&self) -> Result<(), validator::ValidationErrors> {
        Ok(())
    }
}

/// Apply to join a platform flash sale with your products.
///
/// Requires `promotions.participate` on your unit (`org_id`), which must be below the unit that
/// runs the promotion. Each product with a sale price or a discount and the units offered. The
/// platform reviews it; once approved, the products join the flash sale and you pay their
/// discount. Applying again before the review replaces the products (200). Only flash sales that
/// have not started take applications.
#[utoipa::path(
    post,
    path = "/promotions/{id}/participations",
    extensions(("x-permissions" = json!(["promotions.participate"]))),
    operation_id = "apply_to_promotion",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the platform promotion.")),
    request_body = ParticipationRequest,
    responses(
        (status = 201, description = "Applied; waiting for review.", body = Data<ParticipationView>, headers(("Location" = String, description = "Path of the applications of the promotion."))),
        (status = 200, description = "Your waiting application was replaced.", body = Data<ParticipationView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.participate` on your unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion above your unit; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "Not a flash sale taking applications (`promotions.participation_closed`) or already approved (`promotions.participation_already_reviewed`).", body = Problem),
        (status = 422, description = "A product is invalid, not yours or already in the promotion; see `errors`.", body = Problem),
    )
)]
async fn participate(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(body): ValidJson<ParticipationRequest>,
) -> Result<Response, Error> {
    principal.require(&PARTICIPATE)?;
    let location = format!("/api/v1/promotions/promotions/{id}/participations");
    Ok(match service.participate(&tenant, &principal, id, &body).await? {
        Submitted::Created(view) => Created::new(location, view).into_response(),
        Submitted::Replayed(view) => Data { data: view }.into_response(),
    })
}

/// List the applications to a platform promotion.
///
/// Requires `promotions.read` on the promotion's unit (every application) or
/// `promotions.participate` (the applications of your units). Newest first unless `order=asc`;
/// paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/promotions/{id}/participations",
    extensions(("x-permissions" = json!(["promotions.read", "promotions.participate"]))),
    operation_id = "list_promotion_participations",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the platform promotion."), PageQuery),
    responses(
        (status = 200, description = "One page of the applications.", body = Page<ParticipationView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing both `promotions.read` and `promotions.participate`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion; code `promotions.not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_participations(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ParticipationView>, Error> {
    if principal.require(&READ).is_err() {
        principal.require(&PARTICIPATE)?;
    }
    let page = PageRequest::try_from(page)?;
    service.participations(&tenant, &principal, id, &page).await
}

/// Approve a seller's application.
///
/// Requires `promotions.publish` on the promotion's unit. The products join the flash sale at
/// the prices offered; their discount is paid by the seller. Only before the sale starts.
#[utoipa::path(
    post,
    path = "/promotions/{id}/participations/{participation_id}/approve",
    extensions(("x-permissions" = json!(["promotions.publish"]))),
    operation_id = "approve_promotion_participation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the platform promotion."), ("participation_id" = Uuid, Path, description = "Id of the application.")),
    responses(
        (status = 200, description = "Approved.", body = Data<ParticipationView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.publish`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion or application; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "Already approved (`promotions.participation_already_reviewed`) or the sale started (`promotions.participation_closed`).", body = Problem),
    )
)]
async fn approve_participation(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, participation_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<ParticipationView>, Error> {
    principal.require(&PUBLISH)?;
    let view =
        service.review_participation(&tenant, &principal, id, participation_id, true).await?;
    Ok(Data { data: view })
}

/// Reject a seller's application.
///
/// Requires `promotions.publish` on the promotion's unit. The seller can apply again with other
/// products or prices.
#[utoipa::path(
    post,
    path = "/promotions/{id}/participations/{participation_id}/reject",
    extensions(("x-permissions" = json!(["promotions.publish"]))),
    operation_id = "reject_promotion_participation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the platform promotion."), ("participation_id" = Uuid, Path, description = "Id of the application.")),
    responses(
        (status = 200, description = "Rejected.", body = Data<ParticipationView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.publish`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion or application; code `promotions.not_found`.", body = Problem),
        (status = 409, description = "Already approved (`promotions.participation_already_reviewed`) or the sale started (`promotions.participation_closed`).", body = Problem),
    )
)]
async fn reject_participation(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, participation_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<ParticipationView>, Error> {
    principal.require(&PUBLISH)?;
    let view =
        service.review_participation(&tenant, &principal, id, participation_id, false).await?;
    Ok(Data { data: view })
}
