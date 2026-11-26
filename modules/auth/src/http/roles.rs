//! Endpoint role và permission: danh mục permission, CRUD role, role của người dùng.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::{Principal, Scope};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest, page_of};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::permissions::{ROLES_MANAGE, ROLES_READ, USERS_READ};
use crate::role::{Role, RoleInput};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_permissions))
        .routes(routes!(list_roles, create_role))
        .routes(routes!(count_role_members))
        .routes(routes!(get_role, replace_role, delete_role))
        .routes(routes!(get_user_roles, replace_user_roles))
}

/// A permission that roles can grant.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct PermissionResponse {
    /// `<module>.<action>`.
    #[schema(example = "auth.roles.read")]
    name: String,
    /// What the permission allows, in the language of `Accept-Language`.
    description: String,
    /// How careful a grant needs to be: `read`, `write`, `destructive`, `sensitive_data` or
    /// `money`.
    #[schema(example = "read")]
    risk: String,
    /// `true` when the permission applies to data of an organization unit: a role can grant it
    /// for one unit (`unit`), a unit and everything below it (`subtree`), or only the records
    /// a person owns (`own`). `false`: it can only be granted for the whole tenant.
    scoped: bool,
}

/// A permission a role grants, with how far it reaches in the organization tree.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoleGrant {
    #[schema(example = "notes.read")]
    pub permission: String,
    // Mô tả lấy từ chính `Scope` (doc ở đây sẽ biến trường thành `allOf` sai kiểu). Permission
    // không `scoped` chỉ nhận `tenant`.
    #[schema(inline)]
    pub scope: Scope,
}

impl PermissionResponse {
    /// Mọi permission, theo thứ tự tên.
    pub(crate) fn catalog(
        roles: &crate::role::RoleService,
        language: autoxide_kernel::i18n::Language,
    ) -> Vec<PermissionResponse> {
        roles
            .catalog()
            .iter()
            .map(|permission| PermissionResponse {
                name: permission.name().to_owned(),
                description: roles.describe(permission.description(), language),
                risk: permission.risk().as_str().to_owned(),
                scoped: permission.is_scoped(),
            })
            .collect()
    }

    /// One page of the permission catalog, by name. The permissions are declared in code by the modules (a
    /// fixed set per build), so the page is cut in memory (D-20261125-list-pages item 4).
    pub(crate) fn page(
        roles: &crate::role::RoleService,
        language: autoxide_kernel::i18n::Language,
        page: &PageRequest,
    ) -> Result<Page<PermissionResponse>, Error> {
        page_of(Self::catalog(roles, language), |permission| permission.name.clone(), page)
    }
}

/// A role: a named set of permissions given to users.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct RoleResponse {
    id: Uuid,
    #[schema(example = "editor")]
    name: String,
    description: String,
    /// `true` only for the built-in `admin` role: it grants every permission,
    /// including permissions added later, and cannot be changed (except `requires_mfa`) or
    /// deleted.
    all_permissions: bool,
    /// `true`: the role's permissions work only for accounts with two-step sign-in.
    requires_mfa: bool,
    /// Permissions the role grants, sorted by name. Empty when `all_permissions`.
    #[schema(example = json!(["auth.roles.read"]))]
    permissions: Vec<String>,
    /// The same permissions with their scope in the organization tree. Empty when
    /// `all_permissions`.
    grants: Vec<RoleGrant>,
    /// Permissions the role denies, sorted by name; they win over every grant of the user's
    /// other roles.
    denies: Vec<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<Role> for RoleResponse {
    fn from(role: Role) -> Self {
        RoleResponse {
            id: role.id,
            name: role.name,
            description: role.description,
            all_permissions: role.all_permissions,
            requires_mfa: role.requires_mfa,
            permissions: role.permissions,
            grants: role
                .grants
                .into_iter()
                .map(|(permission, scope)| RoleGrant { permission, scope })
                .collect(),
            denies: role.denies,
            created_at: role.created_at,
            updated_at: role.updated_at,
        }
    }
}

/// The full content of a role.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoleRequest {
    /// Unique. Lowercase letters, digits, `_` and `-`, starting with a letter.
    #[schema(example = "editor")]
    name: String,
    /// What the role is for. Optional.
    #[serde(default)]
    description: String,
    /// Permissions to grant; each must be listed by `GET /api/v1/auth/permissions`.
    /// Duplicates are ignored.
    #[serde(default)]
    #[schema(example = json!(["auth.roles.read"]))]
    permissions: Vec<String>,
    /// Permissions to grant with a scope in the organization tree (`own`, `unit`, `subtree`,
    /// `tenant`). A permission is listed once, in `permissions` (whole tenant) or here.
    #[serde(default)]
    #[schema(inline)]
    grants: Vec<RoleGrant>,
    /// Permissions the role denies. A denied permission wins over every grant: no other role of
    /// the user, not even the built-in `admin` role, gives it back. A role either grants or
    /// denies a permission, not both.
    #[serde(default)]
    #[schema(example = json!([]))]
    denies: Vec<String>,
    /// When `true`, the role's permissions work only for accounts with two-step sign-in
    /// (an authenticator app or a passkey); others see `mfa_required` in `GET /api/v1/auth/me`
    /// until they turn it on. The only change the built-in `admin` role accepts.
    #[serde(default)]
    requires_mfa: bool,
}

impl From<RoleRequest> for RoleInput {
    fn from(request: RoleRequest) -> Self {
        RoleInput {
            requires_mfa: request.requires_mfa,
            name: request.name,
            description: request.description,
            permissions: request.permissions,
            grants: request
                .grants
                .into_iter()
                .map(|grant| (grant.permission, grant.scope))
                .collect(),
            denies: request.denies,
        }
    }
}

/// The complete list of roles a user should have.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UserRolesRequest {
    /// Ids of the roles. Replaces every role the user had; `[]` removes all.
    #[validate(length(max = 100))]
    role_ids: Vec<Uuid>,
}

fn roles(roles: Vec<Role>) -> Data<Vec<RoleResponse>> {
    Data { data: roles.into_iter().map(RoleResponse::from).collect() }
}

/// List every permission that roles can grant.
///
/// Requires `auth.roles.read`. Each module declares its own permissions; the list
/// grows when modules are added. Descriptions follow `Accept-Language`. By name (`order=desc`
/// reverses it); paged with `limit` / `cursor`. Without `auth.roles.read`, read your own
/// permissions with `GET /api/v1/me/permissions`.
#[utoipa::path(
    get,
    path = "/permissions",
    extensions(("x-permissions" = json!(["auth.roles.read"]))),
    operation_id = "list_permissions",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of permissions, by name.", body = Page<PermissionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_permissions(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    language: autoxide_kernel::i18n::Language,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<PermissionResponse>, Error> {
    principal.require(&ROLES_READ)?;
    let page = PageRequest::try_from(page)?;
    PermissionResponse::page(&state.roles, language, &page)
}

/// List roles.
///
/// Requires `auth.roles.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/roles",
    extensions(("x-permissions" = json!(["auth.roles.read"]))),
    operation_id = "list_roles",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of roles.", body = Page<RoleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_roles(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<RoleResponse>, Error> {
    principal.require(&ROLES_READ)?;
    let page = state.roles.list(&tenant, &PageRequest::try_from(query)?).await?;
    Ok(page.map(RoleResponse::from))
}

/// How many people hold one role.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct RoleCountResponse {
    role_id: Uuid,
    /// People holding the role, at any unit; each person once. List them with
    /// `GET /api/v1/auth/memberships?role_id=…`.
    members: u64,
}

impl From<(Uuid, usize)> for RoleCountResponse {
    fn from((role_id, members): (Uuid, usize)) -> Self {
        RoleCountResponse { role_id, members: u64::try_from(members).unwrap_or(u64::MAX) }
    }
}

/// Count the people holding each role.
///
/// Requires `auth.roles.read`. One entry per role of the tenant, including roles nobody holds, in
/// the order of `GET /api/v1/auth/roles` (newest first unless `order=asc`); paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get,
    path = "/roles/counts",
    extensions(("x-permissions" = json!(["auth.roles.read"]))),
    operation_id = "count_role_members",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of counts, one entry per role, newest role first.", body = Page<RoleCountResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn count_role_members(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<RoleCountResponse>, Error> {
    principal.require(&ROLES_READ)?;
    let page = PageRequest::try_from(page)?;
    let counts = state.roles.role_member_counts_page(&tenant, &page).await?;
    Ok(counts.map(RoleCountResponse::from))
}

/// Create a role.
///
/// Requires `auth.roles.manage`.
#[utoipa::path(
    post,
    path = "/roles",
    extensions(("x-permissions" = json!(["auth.roles.manage"]))),
    operation_id = "create_role",
    security(("bearer" = [])),
    request_body = RoleRequest,
    responses(
        (status = 201, description = "The role was created.", body = Data<RoleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "Another role has this name; code `auth.role_name_taken`.", body = Problem),
        (status = 422, description = "A value is invalid, e.g. an unknown permission; see `errors`.", body = Problem),
    )
)]
async fn create_role(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<RoleRequest>,
) -> Result<Created<RoleResponse>, Error> {
    principal.require(&ROLES_MANAGE)?;
    let role = state.roles.create(&tenant, Actor::from(&principal), input.into()).await?;
    Ok(Created::new(format!("/auth/roles/{}", role.id), role.into()))
}

/// Get a role.
///
/// Requires `auth.roles.read`.
#[utoipa::path(
    get,
    path = "/roles/{id}",
    extensions(("x-permissions" = json!(["auth.roles.read"]))),
    operation_id = "get_role",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the role.")),
    responses(
        (status = 200, description = "The role.", body = Data<RoleResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No role has this id; code `auth.role_not_found`.", body = Problem),
    )
)]
async fn get_role(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<RoleResponse>, Error> {
    principal.require(&ROLES_READ)?;
    let role = state.roles.get(&tenant, id).await?;
    Ok(Tagged { version: role.updated_at, data: role.into() })
}

/// Replace a role: its name, description and permissions.
///
/// Requires `auth.roles.manage`. You can only add a permission (or widen it, or lift a ban
/// on it) that you have yourself at least as widely. Users with the role get the new
/// permissions on their next request.
#[utoipa::path(
    put,
    path = "/roles/{id}",
    extensions(("x-permissions" = json!(["auth.roles.manage"]))),
    operation_id = "replace_role",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the role."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = RoleRequest,
    responses(
        (status = 200, description = "The role after the change.", body = Data<RoleResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.manage` (code `permission_denied`), or the change adds, widens or unbans a permission you do not have as widely (code `auth.permission_beyond_you`), or turns off `requires_mfa` of a role you could not give (code `auth.role_beyond_you`).", body = Problem),
        (status = 404, description = "No role has this id; code `auth.role_not_found`.", body = Problem),
        (status = 409, description = "Another role has this name (`auth.role_name_taken`), or the role is the built-in `admin` (`auth.role_builtin`).", body = Problem),
        (status = 422, description = "A value is invalid, e.g. an unknown permission; see `errors`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
    )
)]
async fn replace_role(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<RoleRequest>,
) -> Result<Tagged<RoleResponse>, Error> {
    principal.require(&ROLES_MANAGE)?;
    let expected = if_match.expected()?;
    let role = state.roles.update_as(&tenant, &principal, id, input.into(), expected).await?;
    Ok(Tagged { version: role.updated_at, data: role.into() })
}

/// Delete a role.
///
/// Requires `auth.roles.manage`. Users who had the role lose it, so you can only delete a role
/// you could take away: you have its permissions as widely, and every permission it bans.
#[utoipa::path(
    delete,
    path = "/roles/{id}",
    extensions(("x-permissions" = json!(["auth.roles.manage"]))),
    operation_id = "delete_role",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the role.")),
    responses(
        (status = 204, description = "The role was deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.manage` (code `permission_denied`), or the role has permissions you do not have as widely, or bans one you do not have (code `auth.role_beyond_you`).", body = Problem),
        (status = 404, description = "No role has this id; code `auth.role_not_found`.", body = Problem),
        (status = 409, description = "The role is the built-in `admin`; code `auth.role_builtin`.", body = Problem),
    )
)]
async fn delete_role(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&ROLES_MANAGE)?;
    state.roles.delete_as(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the roles of a user.
///
/// Requires `auth.users.read`. By name (`order=desc` reverses it); paged with `limit` /
/// `cursor`.
#[utoipa::path(
    get,
    path = "/users/{id}/roles",
    extensions(("x-permissions" = json!(["auth.users.read"]))),
    operation_id = "get_user_roles",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user."), NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the user's roles, by name.", body = Page<RoleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn get_user_roles(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<RoleResponse>, Error> {
    principal.require(&USERS_READ)?;
    let page = PageRequest::try_from(page)?;
    Ok(state.roles.user_roles_page(&tenant, id, &page).await?.map(RoleResponse::from))
}

/// Replace the roles of a user.
///
/// Requires `auth.roles.manage`. You can only give or take away roles whose permissions you
/// have yourself, at least as widely — for anyone, yourself included. Takes effect on the
/// user's next request. The last user with the `admin` role cannot lose it.
#[utoipa::path(
    put,
    path = "/users/{id}/roles",
    extensions(("x-permissions" = json!(["auth.roles.manage"]))),
    operation_id = "replace_user_roles",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    request_body = UserRolesRequest,
    responses(
        (status = 200, description = "The user's roles after the change, sorted by name.", body = Data<Vec<RoleResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.manage` (code `permission_denied`), or a role given or taken away has permissions you do not have as widely, or a role taken away bans a permission you do not have (code `auth.role_beyond_you`).", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "No user would keep the `admin` role; code `auth.last_admin`.", body = Problem),
        (status = 422, description = "A role id does not exist, or the list is too long; see `errors`.", body = Problem),
    )
)]
async fn replace_user_roles(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UserRolesRequest>,
) -> Result<Data<Vec<RoleResponse>>, Error> {
    principal.require(&ROLES_MANAGE)?;
    Ok(roles(state.roles.set_user_roles_as(&tenant, &principal, id, input.role_ids).await?))
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::event::Actor;
    use autoxide_kernel::pagination::{Order, PageRequest};
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::http::test_support::{PASSWORD, app, bearer, call};

    #[tokio::test]
    async fn chua_co_quyen_thi_bi_tu_choi_admin_thi_lam_duoc_het() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            // Chưa có role nào: đăng nhập được nhưng không làm được gì.
            let (status, _, problem) = call(&app, "GET", "/roles", Some(&ada), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(problem["code"], "permission_denied");
            let (status, _, _) = call(&app, "GET", "/roles", None, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);

            // Cho Ada role `admin` (như lệnh CLI tạo người dùng đầu tiên sẽ làm).
            let page =
                services.roles.list(&tenant, &PageRequest::first(10, Order::Asc)).await.unwrap();
            let admin = page.data.iter().find(|role| role.all_permissions).unwrap();
            let user =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            services
                .roles
                .set_user_roles(&tenant, Actor::system(), user.id, vec![admin.id])
                .await
                .unwrap();

            let (status, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{me}");
            assert_eq!(me["data"]["all_permissions"], true);

            let (status, _, catalog) = call(&app, "GET", "/permissions", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{catalog}");
            assert_eq!(catalog["data"][0]["name"], "auth.api_keys.approve");

            let body = json!({"name": "editor", "permissions": ["notes.write"]});
            let (status, _, created) = call(&app, "POST", "/roles", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            let id = created["data"]["id"].as_str().unwrap().to_owned();

            let (status, _, list) = call(&app, "GET", "/roles?limit=1", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{list}");
            assert_eq!(list["data"][0]["name"], "editor");
            assert!(list["pagination"]["next_cursor"].is_string());

            let body = json!({"name": "writer", "description": "Writes.", "permissions": []});
            let path = format!("/roles/{id}");
            let (status, _, changed) = call(&app, "PUT", &path, Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{changed}");
            assert_eq!(changed["data"]["name"], "writer");

            let user_roles = format!("/users/{}/roles", user.id);
            let body = json!({"role_ids": [admin.id, id]});
            let (status, _, roles) = call(&app, "PUT", &user_roles, Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{roles}");
            assert_eq!(roles["data"].as_array().unwrap().len(), 2);
            let (status, _, roles) = call(&app, "GET", &user_roles, Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{roles}");

            let (status, _, _) = call(&app, "DELETE", &path, Some(&ada), None).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            let (status, _, gone) = call(&app, "GET", &path, Some(&ada), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(gone["code"], "auth.role_not_found");

            let body = json!({"role_ids": []});
            let (status, _, last) = call(&app, "PUT", &user_roles, Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(last["code"], "auth.last_admin");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn role_co_permission_nao_thi_lam_duoc_dung_viec_do() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let reader = services
                .roles
                .create(
                    &tenant,
                    Actor::system(),
                    crate::role::RoleInput {
                        denies: Vec::new(),
                        requires_mfa: false,
                        grants: Vec::new(),
                        name: "reader".to_owned(),
                        description: String::new(),
                        permissions: vec!["auth.roles.read".to_owned()],
                    },
                )
                .await
                .unwrap();
            let user =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            services
                .roles
                .set_user_roles(&tenant, Actor::system(), user.id, vec![reader.id])
                .await
                .unwrap();
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            let (status, _, _) = call(&app, "GET", "/roles", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK);
            let body = json!({"name": "mine", "permissions": []});
            let (status, _, problem) = call(&app, "POST", "/roles", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(
                problem["detail"],
                "This action needs the permission `auth.roles.manage`. Ask an administrator \
                 for a role that has it."
            );
            let (_, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(me["data"]["permissions"], json!(["auth.roles.read"]));
            assert_eq!(me["data"]["all_permissions"], false);
        })
        .await
        .unwrap();
    }

    // Bước 54 mục 1 (P11): quyền không nằm trong access token — đổi role, đổi permission của
    // role có hiệu lực ngay ở request kế tiếp với CÙNG token, không chờ token hết hạn.
    #[tokio::test]
    async fn doi_role_co_hieu_luc_o_request_ke_tiep() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let input = |permissions: &[&str]| crate::role::RoleInput {
                denies: Vec::new(),
                requires_mfa: false,
                grants: Vec::new(),
                name: "reader".to_owned(),
                description: String::new(),
                permissions: permissions.iter().map(|name| (*name).to_owned()).collect(),
            };
            let reader =
                services.roles.create(&tenant, Actor::system(), input(&["auth.roles.read"])).await;
            let reader = reader.unwrap();
            let user =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            let assign = |roles: Vec<uuid::Uuid>| {
                services.roles.set_user_roles(&tenant, Actor::system(), user.id, roles)
            };
            assign(vec![reader.id]).await.unwrap();
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;
            let status = async || call(&app, "GET", "/roles", Some(&ada), None).await.0;
            assert_eq!(status().await, StatusCode::OK);

            // Gỡ role: cùng token, request kế tiếp bị từ chối.
            assign(Vec::new()).await.unwrap();
            assert_eq!(status().await, StatusCode::FORBIDDEN);
            // Gán lại: có quyền lại ngay.
            assign(vec![reader.id]).await.unwrap();
            assert_eq!(status().await, StatusCode::OK);
            // Bớt permission của role (người dùng vẫn giữ role): mất quyền ngay.
            services
                .roles
                .update(&tenant, Actor::system(), reader.id, input(&[]), None)
                .await
                .unwrap();
            assert_eq!(status().await, StatusCode::FORBIDDEN);
        })
        .await
        .unwrap();
    }

    // PT-9 (D-20261126-no-self-grant): người quản lý role không tự gán `admin` cho mình qua REST.
    #[tokio::test]
    async fn quan_ly_role_khong_tu_gan_admin() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let tenant = services.tenant.clone();
            let manager = crate::role::RoleInput {
                denies: Vec::new(),
                requires_mfa: false,
                grants: Vec::new(),
                name: "manager".to_owned(),
                description: String::new(),
                permissions: vec!["auth.roles.manage".to_owned(), "auth.roles.read".to_owned()],
            };
            let manager = services.roles.create(&tenant, Actor::system(), manager).await.unwrap();
            let page =
                services.roles.list(&tenant, &PageRequest::first(10, Order::Asc)).await.unwrap();
            let admin = page.data.iter().find(|role| role.all_permissions).unwrap().id;
            let ada =
                services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
            services
                .roles
                .set_user_roles(&tenant, Actor::system(), ada.id, vec![manager.id])
                .await
                .unwrap();
            let token = bearer(&app, "ada@example.com", PASSWORD).await;
            let body = json!({"role_ids": [manager.id, admin]});
            let path = format!("/users/{}/roles", ada.id);
            let (status, _, problem) = call(&app, "PUT", &path, Some(&token), Some(body)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
            assert_eq!(problem["code"], "auth.role_beyond_you");
        })
        .await
        .unwrap();
    }
}
