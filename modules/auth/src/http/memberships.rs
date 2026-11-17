//! Membership theo nút của cây tổ chức (bước 34 mục 2): người làm ở nút nào với role nào.
//! `PUT /users/{id}/roles` cũ vẫn chạy: nó là membership ở nút gốc.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use super::roles::RoleResponse;
use crate::permissions::{ROLES_MANAGE, USERS_READ};
use crate::role::{Membership, MembershipFilter, MembershipRow};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(get_user_memberships, replace_user_memberships))
        .routes(routes!(list_memberships))
        .routes(routes!(count_memberships))
        .routes(routes!(get_my_memberships))
}

/// The roles a user has in one organization unit.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MembershipResponse {
    /// The unit; the root unit has the id of the tenant.
    org_id: Uuid,
    /// Name of the unit, so a user who cannot read the organization tree still sees where they
    /// work; `null` when the unit no longer exists (or no organization module is installed).
    #[schema(example = "Store 1")]
    org_name: Option<String>,
    /// Type of the unit (see `GET /api/v1/organizations/unit-types`); `tenant` for the root, `null`
    /// like `org_name`.
    #[schema(example = "store")]
    org_unit_type: Option<String>,
    /// The roles there, sorted by name. A role grants each permission at its scope from this
    /// unit (`unit`: this unit, `subtree`: this unit and the units below it).
    roles: Vec<RoleResponse>,
}

impl From<Membership> for MembershipResponse {
    fn from(membership: Membership) -> Self {
        MembershipResponse {
            org_id: membership.org_id,
            org_name: None,
            org_unit_type: None,
            roles: membership.roles.into_iter().map(RoleResponse::from).collect(),
        }
    }
}

/// One role a user has in one unit.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MembershipRowResponse {
    id: Uuid,
    user_id: Uuid,
    /// The unit of the membership.
    org_id: Uuid,
    role_id: Uuid,
    #[schema(example = "store_manager")]
    role_name: String,
    /// `true` when the membership is on a unit above the one asked for with `org_id`: the user
    /// works there through it.
    inherited: bool,
    created_at: DateTime<Utc>,
}

impl From<MembershipRow> for MembershipRowResponse {
    fn from(row: MembershipRow) -> Self {
        MembershipRowResponse {
            id: row.id,
            user_id: row.user_id,
            org_id: row.org_id,
            role_id: row.role_id,
            role_name: row.role_name,
            inherited: row.inherited,
            created_at: row.created_at,
        }
    }
}

/// The roles to give in one unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MembershipRequest {
    /// The unit (see `GET /api/v1/organizations/units`); the id of the tenant is the root.
    org_id: Uuid,
    /// Ids of the roles in this unit.
    role_ids: Vec<Uuid>,
}

/// Every unit a user should work in, with the roles there.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MembershipsRequest {
    /// Replaces every membership the user had; `[]` removes all.
    memberships: Vec<MembershipRequest>,
}

/// Which memberships to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct MembershipsQuery {
    /// Only memberships on this unit (with `inherited=true`: also on the units above it).
    org_id: Option<Uuid>,
    /// Only memberships with this role.
    role_id: Option<Uuid>,
    /// Only the memberships of this user.
    user_id: Option<Uuid>,
    /// With `org_id`: also list memberships on the units above it, which reach it through
    /// `subtree` or `tenant` scopes. Default `false`.
    #[serde(default)]
    inherited: bool,
}

/// Câu trả lời kèm tên nút: một lần tra mỗi nút qua `OrgDirectory` (người dùng làm ở ít nút).
pub(crate) async fn memberships(
    state: &HttpState,
    tenant: &TenantContext,
    list: Vec<Membership>,
) -> Result<Data<Vec<MembershipResponse>>, Error> {
    let mut data = Vec::with_capacity(list.len());
    for membership in list {
        let mut response = MembershipResponse::from(membership);
        if let Some(orgs) = &state.orgs {
            let id = autoxide_kernel::tenancy::OrgId::from_uuid(response.org_id);
            if let Some(unit) = orgs.unit(tenant, id).await? {
                response.org_name = Some(unit.name);
                response.org_unit_type = Some(unit.unit_type);
            }
        }
        data.push(response);
    }
    Ok(Data { data })
}

/// List the organization units a user works in, with the roles there.
///
/// Requires `auth.users.read`.
#[utoipa::path(
    get,
    path = "/users/{id}/memberships",
    extensions(("x-permissions" = json!(["auth.users.read"]))),
    operation_id = "get_user_memberships",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 200, description = "One entry per unit, sorted by unit id.", body = Data<Vec<MembershipResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
    )
)]
async fn get_user_memberships(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<MembershipResponse>>, Error> {
    principal.require(&USERS_READ)?;
    let list = state.roles.memberships(&tenant, id).await?;
    memberships(&state, &tenant, list).await
}

/// Replace every organization unit a user works in, and the roles there.
///
/// Requires `auth.roles.manage`. You can only give roles whose permissions you have yourself,
/// in units you work in; the built-in `admin` role only at the root unit. Takes effect on the
/// user's next request. The last user with the `admin` role cannot lose it.
#[utoipa::path(
    put,
    path = "/users/{id}/memberships",
    extensions(("x-permissions" = json!(["auth.roles.manage"]))),
    operation_id = "replace_user_memberships",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    request_body = MembershipsRequest,
    responses(
        (status = 200, description = "The user's memberships after the change.", body = Data<Vec<MembershipResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "The change would leave no active admin; code `auth.last_admin`.", body = Problem),
        (status = 422, description = "A unit or role is unknown, outside your scope, listed twice, or a role has permissions you do not have; see `errors`.", body = Problem),
    )
)]
async fn replace_user_memberships(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MembershipsRequest>,
) -> Result<Data<Vec<MembershipResponse>>, Error> {
    principal.require(&ROLES_MANAGE)?;
    let wanted =
        input.memberships.into_iter().map(|entry| (entry.org_id, entry.role_ids)).collect();
    let list = state.roles.set_memberships(&tenant, &principal, id, wanted).await?;
    memberships(&state, &tenant, list).await
}

/// List who works in which organization unit, with which role.
///
/// Requires `auth.users.read`. Filter by unit, role or user; with `org_id` and `inherited=true`,
/// memberships on the units above also count (marked `inherited`). Newest first unless
/// `order=asc`.
#[utoipa::path(
    get,
    path = "/memberships",
    extensions(("x-permissions" = json!(["auth.users.read"]))),
    operation_id = "list_memberships",
    security(("bearer" = [])),
    params(MembershipsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of memberships.", body = Page<MembershipRowResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid, e.g. `org_id` is not a UUID; code `validation_failed`, every problem in `errors`.", body = Problem),
    )
)]
async fn list_memberships(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<MembershipsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<MembershipRowResponse>, Error> {
    principal.require(&USERS_READ)?;
    let filter = MembershipFilter {
        org_id: filter.org_id,
        role_id: filter.role_id,
        user_id: filter.user_id,
        inherited: filter.inherited,
    };
    let page =
        state.roles.list_memberships(&tenant, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(MembershipRowResponse::from))
}

/// How many people work in one organization unit.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MemberCountResponse {
    org_id: Uuid,
    /// People with a membership on this unit itself (each person once, whatever their roles).
    direct: u64,
    /// People with a membership on this unit or on a unit above it — the people
    /// `GET /api/v1/auth/memberships?org_id=…&inherited=true` lists.
    total: u64,
}

/// Count the people of every organization unit.
///
/// Requires `auth.users.read`. One entry per unit of the tree, the root first, including units
/// nobody works in — for showing a number next to each unit without reading every membership.
#[utoipa::path(
    get,
    path = "/memberships/counts",
    extensions(("x-permissions" = json!(["auth.users.read"]))),
    operation_id = "count_memberships",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The counts, one entry per unit.", body = Data<Vec<MemberCountResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn count_memberships(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<MemberCountResponse>>, Error> {
    principal.require(&USERS_READ)?;
    let counts = state.roles.member_counts(&tenant).await?;
    Ok(Data { data: counts.into_iter().map(MemberCountResponse::from).collect() })
}

impl From<crate::role::MemberCount> for MemberCountResponse {
    fn from(count: crate::role::MemberCount) -> Self {
        let to_u64 = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
        MemberCountResponse {
            org_id: count.org_id,
            direct: to_u64(count.direct),
            total: to_u64(count.total),
        }
    }
}

/// List the organization units you work in, with your roles there.
///
/// Any signed-in user.
#[utoipa::path(
    get,
    path = "/me/memberships",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_memberships",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "One entry per unit, sorted by unit id.", body = Data<Vec<MembershipResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn get_my_memberships(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<MembershipResponse>>, Error> {
    let list = state.roles.memberships(&tenant, principal.user_id()).await?;
    memberships(&state, &tenant, list).await
}
