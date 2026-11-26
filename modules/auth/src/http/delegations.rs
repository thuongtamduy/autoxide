//! Uỷ quyền theo cây khách hàng (D-310) và danh sách khách người gọi vào được (`/me/tenants`).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest, page_of};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::delegation::{Delegation, NewDelegation};
use crate::permissions::{DELEGATIONS_MANAGE, DELEGATIONS_READ};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_delegations, grant_delegation))
        .routes(routes!(revoke_delegation))
        .routes(routes!(my_tenants))
        .routes(routes!(list_child_tenants))
}

/// A delegation: holders of a role of the parent tenant work in a child tenant with it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct DelegationResponse {
    id: Uuid,
    /// The tenant that granted it (owner of the role).
    parent_tenant_id: Uuid,
    /// The tenant they may enter, with the header `Autoxide-Tenant`.
    child_tenant_id: Uuid,
    /// The role of the parent tenant; its holders (at the parent's root unit) get its
    /// permissions in the child.
    role_id: Uuid,
    /// The unit of the child tenant the permissions apply at.
    org_id: Uuid,
    reason: String,
    /// When it stops working by itself; `null`: until revoked.
    expires_at: Option<Timestamp>,
    /// When it was revoked; `null` while in force.
    revoked_at: Option<Timestamp>,
    created_at: Timestamp,
}

impl From<Delegation> for DelegationResponse {
    fn from(row: Delegation) -> Self {
        DelegationResponse {
            id: row.id,
            parent_tenant_id: row.tenant_id,
            child_tenant_id: row.child_tenant_id,
            role_id: row.role_id,
            org_id: row.org_id,
            reason: row.reason,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            created_at: row.created_at,
        }
    }
}

/// Which delegations to list.
#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Direction {
    /// Given by this tenant to its children (default).
    #[default]
    Outgoing,
    /// Received from the tenants above this one.
    Incoming,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DelegationQuery {
    /// `outgoing` (default) or `incoming`.
    #[param(inline)]
    #[serde(default)]
    direction: Direction,
}

/// A new delegation to a child tenant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DelegationRequest {
    /// A tenant below this one (its child, or a child of a child).
    child_tenant_id: Uuid,
    /// A role of this tenant. A role with every permission can only be delegated at the root
    /// unit of the child.
    role_id: Uuid,
    /// The unit of the child tenant the permissions apply at; omit for its root unit.
    org_id: Option<Uuid>,
    /// When it stops working by itself; omit for "until revoked".
    expires_at: Option<Timestamp>,
    /// Why, 1 to 500 characters; kept in both audit logs.
    reason: String,
}

/// List the delegations of this tenant.
///
/// Requires `auth.delegations.read`. `outgoing`: given to child tenants; `incoming`: received
/// from the tenants above. Newest first unless `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/delegations",
    extensions(("x-permissions" = json!(["auth.delegations.read"]))),
    operation_id = "list_delegations",
    security(("bearer" = [])),
    params(DelegationQuery, PageQuery),
    responses(
        (status = 200, description = "One page of delegations, newest first.", body = Page<DelegationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.delegations.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_delegations(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<DelegationQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<DelegationResponse>, Error> {
    principal.require(&DELEGATIONS_READ)?;
    let page = PageRequest::try_from(page)?;
    let rows = match query.direction {
        Direction::Outgoing => state.delegations.outgoing(&tenant, &page).await?,
        Direction::Incoming => state.delegations.incoming(&tenant, &page).await?,
    };
    Ok(rows.map(Into::into))
}

/// A tenant below this one (a child, or a child of a child…).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct ChildTenantResponse {
    id: Uuid,
    /// The tenant right above it (this tenant for a direct child).
    parent_id: Uuid,
    #[schema(example = "store-hanoi")]
    slug: String,
    name: String,
    /// `provisioning`, `active`, `suspended`, `maintenance` or `moving`.
    #[schema(example = "active")]
    status: String,
}

/// List the tenants below this one, to choose where a delegation goes.
///
/// Requires `auth.delegations.manage`. Children first, then their children, each level sorted
/// by slug; at most 16 levels down (`order=desc` reverses the whole list); paged with `limit` /
/// `cursor`. Only names and states — nothing of their data.
#[utoipa::path(
    get,
    path = "/delegations/tenants",
    extensions(("x-permissions" = json!(["auth.delegations.manage"]))),
    operation_id = "list_child_tenants",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the tenants below this one, level by level.", body = Page<ChildTenantResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.delegations.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_child_tenants(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<ChildTenantResponse>, Error> {
    principal.require(&DELEGATIONS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    // Cây khách đọc từng tầng (tối đa 16 tầng) trên sổ khách của mặt điều khiển; thứ tự (tầng, slug) chỉ biết
    // sau khi đi hết các tầng trên, nên phân trang trong bộ nhớ theo vị trí trong lần đi (D-20261125-list-pages
    // mục 4, như tổ tiên của nút `organizations`). Khoá là vị trí, đủ 6 chữ số để so theo byte đúng thứ tự.
    let children = state.delegations.databases.tenant_descendants(tenant.id()).await?;
    let keyed: Vec<(String, ChildTenantResponse)> = children
        .into_iter()
        .enumerate()
        .map(|(index, child)| {
            let response = ChildTenantResponse {
                id: child.id.as_uuid(),
                parent_id: child.parent_id.as_uuid(),
                slug: child.slug,
                name: child.name,
                status: child.status,
            };
            (format!("{index:06}"), response)
        })
        .collect();
    Ok(page_of(keyed, |(key, _)| key.clone(), &page)?.map(|(_, child)| child))
}

/// Let holders of a role work in a child tenant.
///
/// Requires `auth.delegations.manage` in the parent tenant. People holding the role at this
/// tenant's root unit then call the API with their usual credentials and the header
/// `Autoxide-Tenant: <child id>`; they get the role's permissions at the given unit of the
/// child. Every change they make there is recorded with the delegation as the way in.
/// Revoking takes effect at their next request.
#[utoipa::path(
    post,
    path = "/delegations",
    extensions(("x-permissions" = json!(["auth.delegations.manage"]))),
    operation_id = "grant_delegation",
    security(("bearer" = [])),
    request_body = DelegationRequest,
    responses(
        (status = 201, description = "The delegation is in force.", body = Data<DelegationResponse>, headers(("Location" = String, description = "Address of the new delegation."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.delegations.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "The child is not below this tenant, the role or unit does not exist, the expiry is past, or the reason is missing; see `errors`.", body = Problem),
    )
)]
async fn grant_delegation(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<DelegationRequest>,
) -> Result<Created<DelegationResponse>, Error> {
    principal.require(&DELEGATIONS_MANAGE)?;
    let input = NewDelegation {
        child_tenant_id: input.child_tenant_id,
        role_id: input.role_id,
        org_id: input.org_id,
        expires_at: input.expires_at,
        reason: input.reason,
    };
    let row = state.delegations.grant(&tenant, &principal, input).await?;
    let path = format!("/auth/delegations/{}", row.id);
    Ok(Created::new(path, row.into()))
}

/// Revoke a delegation.
///
/// Requires `auth.delegations.manage` in the parent tenant that granted it. The people it let
/// in lose access at their next request. Revoking again changes nothing.
#[utoipa::path(
    delete,
    path = "/delegations/{id}",
    extensions(("x-permissions" = json!(["auth.delegations.manage"]))),
    operation_id = "revoke_delegation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the delegation.")),
    responses(
        (status = 200, description = "The delegation, revoked.", body = Data<DelegationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.delegations.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "This tenant granted no delegation with this id; code `auth.delegation_not_found`.", body = Problem),
    )
)]
async fn revoke_delegation(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<DelegationResponse>, Error> {
    principal.require(&DELEGATIONS_MANAGE)?;
    let row = state.delegations.revoke(&tenant, &principal, id).await?;
    Ok(Data { data: row.into() })
}

/// A tenant you can work in.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MyTenantResponse {
    /// Send it in the header `Autoxide-Tenant` to work there.
    id: Uuid,
    #[schema(example = "acme-hanoi")]
    slug: String,
    name: String,
    /// `direct` (your own tenant), `delegation` (a parent tenant let your role in) or
    /// `support` (time-limited platform support access).
    #[schema(example = "direct")]
    access: String,
    /// The delegation or support grant; `null` for `direct`.
    access_id: Option<Uuid>,
}

/// List the tenants you can work in.
///
/// Any signed-in user. Your own tenant first, then the ones you may enter through a delegation,
/// then through a support grant (send the header `Autoxide-Tenant: <id>` with your usual
/// credentials); `order=desc` reverses the list; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/me/tenants",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_tenants",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the tenants, your own first.", body = Page<MyTenantResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn my_tenants(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<MyTenantResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    // Khách nhà: khách của credential, kể cả khi request đang vào khách khác.
    let home_id = principal.operator_from().unwrap_or(principal.tenant_id());
    let home = state.delegations.databases.tenant(home_id).await?;
    let mut data = vec![MyTenantResponse {
        id: home.id().as_uuid(),
        slug: home.slug().to_owned(),
        name: home.name().to_owned(),
        access: "direct".to_owned(),
        access_id: None,
    }];
    if principal.realm() == autoxide_kernel::access::Realm::Staff {
        for delegation in state.delegations.usable(&home, principal.user_id(), None).await? {
            if data.iter().any(|known| known.id == delegation.child_tenant_id) {
                continue;
            }
            let Ok(child) = state
                .delegations
                .databases
                .tenant(TenantId::from_uuid(delegation.child_tenant_id))
                .await
            else {
                continue;
            };
            data.push(MyTenantResponse {
                id: child.id().as_uuid(),
                slug: child.slug().to_owned(),
                name: child.name().to_owned(),
                access: "delegation".to_owned(),
                access_id: Some(delegation.id),
            });
        }
        // Nhân sự nền tảng: khách có lần cấp quyền vào hỗ trợ còn hạn (D-313).
        if home.is_platform() {
            for grant in state.support.live_for(principal.user_id(), None).await? {
                let target = TenantId::from_uuid(grant.target_tenant_id);
                if data.iter().any(|known| known.id == grant.target_tenant_id) {
                    continue;
                }
                let Ok(found) = state.delegations.databases.tenant(target).await else { continue };
                data.push(MyTenantResponse {
                    id: found.id().as_uuid(),
                    slug: found.slug().to_owned(),
                    name: found.name().to_owned(),
                    access: "support".to_owned(),
                    access_id: Some(grant.id),
                });
            }
        }
    }
    // Gộp từ nhiều nguồn và nhiều database (khách nhà, uỷ quyền ở database của khách nhà, lần cấp hỗ trợ ở
    // khách nền tảng; tên mỗi khách tra sổ khách): phân trang trong bộ nhớ (D-20261125-list-pages mục 4, như
    // độ trễ `reporting`). Khoá giữ thứ tự: hạng đường vào (0 nhà, 1 uỷ quyền, 2 hỗ trợ) rồi id khách — mỗi
    // khách chỉ một dòng.
    page_of(
        data,
        |tenant| {
            let rank = match tenant.access.as_str() {
                "direct" => 0,
                "delegation" => 1,
                _ => 2,
            };
            format!("{rank}{}", tenant.id)
        },
        &page,
    )
}
