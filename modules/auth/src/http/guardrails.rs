//! Endpoint rào quyền tối đa cho khách con (bước 54).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::guardrail::GuardrailView;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_guardrails))
        .routes(routes!(applied_guardrails))
        .routes(routes!(set_guardrail, remove_guardrail))
}

/// The most permissions a tenant below this one can use.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct GuardrailResponse {
    /// The tenant limited by the guardrail (and every tenant below it).
    tenant_id: Uuid,
    /// The tenant above it that set the guardrail.
    set_by_tenant_id: Uuid,
    /// The permissions roles there may still grant, sorted by name. Any other permission is
    /// refused with `permission_denied` and `reason: outside_guardrail`, even for administrators.
    #[schema(example = json!(["content.read", "content.write"]))]
    max_permissions: Vec<String>,
    /// Who set it last, and when.
    updated_by: Uuid,
    updated_at: Timestamp,
}

impl From<GuardrailView> for GuardrailResponse {
    fn from(view: GuardrailView) -> Self {
        GuardrailResponse {
            tenant_id: view.tenant_id,
            set_by_tenant_id: view.set_by_tenant_id,
            max_permissions: view.max_permissions,
            updated_by: view.updated_by,
            updated_at: view.updated_at,
        }
    }
}

/// The guardrails that limit this tenant.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct AppliedGuardrailsResponse {
    /// The most permissions anyone can use in this tenant: what every guardrail allows together,
    /// sorted by name. `null` when no guardrail limits this tenant.
    #[schema(example = json!(["content.read"]))]
    max_permissions: Option<Vec<String>>,
    /// Every guardrail on the way from the top tenant to this one.
    guardrails: Vec<GuardrailResponse>,
}

/// A guardrail to set.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct GuardrailRequest {
    /// The permissions roles of that tenant may grant at most (see
    /// `GET /api/v1/auth/permissions`). `[]` leaves no permission.
    #[schema(example = json!(["content.read", "content.write"]))]
    max_permissions: Vec<String>,
}

/// List the guardrails this tenant set for tenants below it.
///
/// Requires `auth.guardrails.manage`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/guardrails",
    extensions(("x-permissions" = json!(["auth.guardrails.manage"]))),
    operation_id = "list_guardrails",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of guardrails.", body = Page<GuardrailResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.guardrails.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; code `validation_failed`.", body = Problem),
    )
)]
async fn list_guardrails(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<GuardrailResponse>, Error> {
    let page = PageRequest::try_from(query)?;
    Ok(state.guardrails.list_set(&principal, &page).await?.map(GuardrailResponse::from))
}

/// See the guardrails that limit this tenant.
///
/// Requires `auth.roles.read`. Tenants above this one may set the most permissions anyone here can
/// use; this lists them and what they allow together.
#[utoipa::path(
    get,
    path = "/guardrails/applied",
    extensions(("x-permissions" = json!(["auth.roles.read"]))),
    operation_id = "applied_guardrails",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The guardrails on this tenant.", body = Data<AppliedGuardrailsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn applied_guardrails(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
) -> Result<Data<AppliedGuardrailsResponse>, Error> {
    let (rails, allowed) = state.guardrails.applied(&principal).await?;
    let guardrails = rails.into_iter().map(GuardrailResponse::from).collect();
    Ok(Data { data: AppliedGuardrailsResponse { max_permissions: allowed, guardrails } })
}

/// Set the most permissions a tenant below this one can use.
///
/// Requires `auth.guardrails.manage`, from this tenant's own accounts (not while working in another
/// tenant). The tenant must be below this one in the customer tree. Takes effect for that tenant
/// and every tenant below it, for every role including administrators, within 30 seconds on every
/// server. Guardrails set by several tenants above all apply: a tenant cannot widen what a tenant
/// above it allowed.
#[utoipa::path(
    put,
    path = "/guardrails/{tenant_id}",
    extensions(("x-permissions" = json!(["auth.guardrails.manage"]))),
    operation_id = "set_guardrail",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant below this one.")),
    request_body = GuardrailRequest,
    responses(
        (status = 200, description = "The guardrail as set.", body = Data<GuardrailResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.guardrails.manage` (code `permission_denied`), or the tenant is not below this one (code `auth.guardrail_not_a_child`).", body = Problem),
        (status = 422, description = "A permission is unknown; code `validation_failed`, see `errors`.", body = Problem),
    )
)]
async fn set_guardrail(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(child): Path<Uuid>,
    ValidJson(input): ValidJson<GuardrailRequest>,
) -> Result<Data<GuardrailResponse>, Error> {
    let view = state.guardrails.set(&tenant, &principal, child, input.max_permissions).await?;
    Ok(Data { data: view.into() })
}

/// Remove the guardrail this tenant set for a tenant below it.
///
/// Requires `auth.guardrails.manage`. Guardrails set by other tenants above stay.
#[utoipa::path(
    delete,
    path = "/guardrails/{tenant_id}",
    extensions(("x-permissions" = json!(["auth.guardrails.manage"]))),
    operation_id = "remove_guardrail",
    security(("bearer" = [])),
    params(("tenant_id" = Uuid, Path, description = "Id of the tenant below this one.")),
    responses(
        (status = 204, description = "The guardrail was removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.guardrails.manage` (code `permission_denied`), or the tenant is not below this one (code `auth.guardrail_not_a_child`).", body = Problem),
        (status = 404, description = "This tenant set no guardrail for it; code `auth.guardrail_not_found`.", body = Problem),
    )
)]
async fn remove_guardrail(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(child): Path<Uuid>,
) -> Result<StatusCode, Error> {
    state.guardrails.remove(&tenant, &principal, child).await?;
    Ok(StatusCode::NO_CONTENT)
}
