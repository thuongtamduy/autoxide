//! Quyền vào hỗ trợ của nhân sự nền tảng (D-190, D-211, D-313): phía khách xem, tự thu hồi; phía
//! nền tảng cấp, xem, thu hồi.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::permissions::{SUPPORT_ACCESS_READ, SUPPORT_ACCESS_REVOKE, SUPPORT_GRANT};
use crate::support::{NewSupportGrant, SupportGrant};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_support_access))
        .routes(routes!(revoke_support_access))
        .routes(routes!(list_support_grants, grant_support))
        .routes(routes!(revoke_support_grant))
}

/// A time-limited access of platform staff to a tenant's data.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct SupportGrantResponse {
    id: Uuid,
    /// The tenant the staff may enter.
    tenant_id: Uuid,
    /// The platform staff account.
    staff_user_id: Uuid,
    /// Display name of that staff member, so the tenant knows who may enter; `null` when the
    /// account no longer exists.
    #[schema(example = "Lan Nguyen")]
    staff_name: Option<String>,
    /// The role of the platform tenant whose permissions they get.
    role_id: Uuid,
    /// `support`, or `legal_request` (a request of a competent authority; the tenant cannot
    /// revoke it).
    #[schema(example = "support")]
    basis: String,
    reason: String,
    /// Number of the authority's document, for `legal_request`.
    legal_reference: Option<String>,
    granted_by: Option<Uuid>,
    starts_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    /// When the staff first entered under it; `null`: not used.
    first_used_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    /// `true` when the tenant revoked it.
    revoked_by_tenant: bool,
    created_at: DateTime<Utc>,
}

impl From<SupportGrant> for SupportGrantResponse {
    fn from(row: SupportGrant) -> Self {
        SupportGrantResponse {
            id: row.id,
            tenant_id: row.target_tenant_id,
            staff_user_id: row.staff_user_id,
            staff_name: None,
            role_id: row.role_id,
            basis: row.basis,
            reason: row.reason,
            legal_reference: row.legal_reference,
            granted_by: row.granted_by,
            starts_at: row.starts_at,
            expires_at: row.expires_at,
            first_used_at: row.first_used_at,
            revoked_at: row.revoked_at,
            revoked_by_tenant: row.revoked_by_tenant,
            created_at: row.created_at,
        }
    }
}

/// A new support access grant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SupportGrantRequest {
    /// The tenant to enter.
    tenant_id: Uuid,
    /// A staff account of the platform tenant.
    staff_user_id: Uuid,
    /// A role of the platform tenant: its permissions apply in the tenant, at its root unit.
    role_id: Uuid,
    /// `support` (the tenant must allow it with the setting `auth.support_access`) or
    /// `legal_request` (a request of a competent authority; needs `legal_reference`).
    #[schema(example = "support")]
    basis: String,
    /// Why, 10 to 500 characters; the tenant sees it.
    reason: String,
    /// For `legal_request`: number of the authority's document.
    legal_reference: Option<String>,
    /// How long, 1 to `AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS` hours.
    #[schema(example = 4)]
    hours: u32,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TenantQuery {
    /// The tenant the grants are for.
    tenant_id: Uuid,
}

/// Câu trả lời kèm tên nhân sự (một câu cho mọi dòng).
async fn described(
    state: &HttpState,
    rows: Vec<SupportGrant>,
) -> Result<Vec<SupportGrantResponse>, Error> {
    let names = state.support.staff_names(rows.iter().map(|row| row.staff_user_id)).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let name = names.get(&row.staff_user_id).cloned();
            SupportGrantResponse { staff_name: name, ..row.into() }
        })
        .collect())
}

async fn described_one(
    state: &HttpState,
    row: SupportGrant,
) -> Result<SupportGrantResponse, Error> {
    let mut list = described(state, vec![row]).await?;
    list.pop().ok_or_else(|| Error::internal("a described grant went missing"))
}

/// List platform staff access to this tenant's data.
///
/// Requires `auth.support_access.read`. Every grant, newest first, with when it was first used
/// and whether it was revoked. Changes made under a grant are in the audit log with
/// `access.kind = support`.
#[utoipa::path(
    get,
    path = "/support-access",
    extensions(("x-permissions" = json!(["auth.support_access.read"]))),
    operation_id = "list_support_access",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The grants.", body = Data<Vec<SupportGrantResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.support_access.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_support_access(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<SupportGrantResponse>>, Error> {
    principal.require(&SUPPORT_ACCESS_READ)?;
    let rows = state.support.for_tenant(tenant.id()).await?;
    Ok(Data { data: described(&state, rows).await? })
}

/// Revoke a platform staff access to this tenant.
///
/// Requires `auth.support_access.revoke`. Takes effect at their next request. Access granted
/// for a request of a competent authority cannot be revoked here.
#[utoipa::path(
    delete,
    path = "/support-access/{id}",
    extensions(("x-permissions" = json!(["auth.support_access.revoke"]))),
    operation_id = "revoke_support_access",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the grant.")),
    responses(
        (status = 200, description = "The grant, revoked.", body = Data<SupportGrantResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), or it was granted for a request of a competent authority (`auth.support_grant_legal_request`).", body = Problem),
        (status = 404, description = "No grant with this id for this tenant; code `auth.support_grant_not_found`.", body = Problem),
    )
)]
async fn revoke_support_access(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SupportGrantResponse>, Error> {
    principal.require(&SUPPORT_ACCESS_REVOKE)?;
    let row = state.support.revoke(tenant.id(), &principal, id, true).await?;
    Ok(Data { data: described_one(&state, row).await? })
}

/// List the support access grants for a tenant (platform).
///
/// Requires `auth.support.grant`, in the platform tenant.
#[utoipa::path(
    get,
    path = "/support-grants",
    extensions(("x-permissions" = json!(["auth.support.grant"]))),
    operation_id = "list_support_grants",
    security(("bearer" = [])),
    params(TenantQuery),
    responses(
        (status = 200, description = "The grants, newest first.", body = Data<Vec<SupportGrantResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.support.grant`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_support_grants(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Query(query): Query<TenantQuery>,
) -> Result<Data<Vec<SupportGrantResponse>>, Error> {
    principal.require(&SUPPORT_GRANT)?;
    let rows = state.support.for_tenant(TenantId::from_uuid(query.tenant_id)).await?;
    Ok(Data { data: described(&state, rows).await? })
}

/// Grant platform staff time-limited access to a tenant's data (platform).
///
/// Requires `auth.support.grant`, in the platform tenant. The staff member then calls the API
/// with their own credentials and the header `Autoxide-Tenant: <tenant id>`, with the role's
/// permissions, until it expires or is revoked. The tenant sees the grant, each first use, and
/// every change made, in its audit log.
#[utoipa::path(
    post,
    path = "/support-grants",
    extensions(("x-permissions" = json!(["auth.support.grant"]))),
    operation_id = "grant_support_access",
    security(("bearer" = [])),
    request_body = SupportGrantRequest,
    responses(
        (status = 201, description = "The grant is in force.", body = Data<SupportGrantResponse>, headers(("Location" = String, description = "Address of the grant."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), not called in the platform tenant (`auth.support_platform_only`), or the tenant turned support access off and the basis is not `legal_request` (`auth.support_access_disabled`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or the duration is too long (`auth.support_duration_exceeded`).", body = Problem),
    )
)]
async fn grant_support(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    ValidJson(input): ValidJson<SupportGrantRequest>,
) -> Result<Created<SupportGrantResponse>, Error> {
    principal.require(&SUPPORT_GRANT)?;
    let input = NewSupportGrant {
        tenant_id: input.tenant_id,
        staff_user_id: input.staff_user_id,
        role_id: input.role_id,
        basis: input.basis,
        reason: input.reason,
        legal_reference: input.legal_reference,
        hours: input.hours,
    };
    let row = state.support.grant(&principal, input).await?;
    let path = format!("/auth/support-grants/{}", row.id);
    Ok(Created::new(path, described_one(&state, row).await?))
}

/// Revoke a support access grant (platform).
///
/// Requires `auth.support.grant`, in the platform tenant. Takes effect at the staff member's
/// next request.
#[utoipa::path(
    delete,
    path = "/support-grants/{id}",
    extensions(("x-permissions" = json!(["auth.support.grant"]))),
    operation_id = "revoke_support_grant",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the grant."), TenantQuery),
    responses(
        (status = 200, description = "The grant, revoked.", body = Data<SupportGrantResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.support.grant`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No grant with this id for this tenant; code `auth.support_grant_not_found`.", body = Problem),
    )
)]
async fn revoke_support_grant(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<TenantQuery>,
) -> Result<Data<SupportGrantResponse>, Error> {
    principal.require(&SUPPORT_GRANT)?;
    let target = TenantId::from_uuid(query.tenant_id);
    let row = state.support.revoke(target, &principal, id, false).await?;
    Ok(Data { data: described_one(&state, row).await? })
}
