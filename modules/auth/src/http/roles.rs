//! Endpoint role và permission: danh mục permission, CRUD role, role của người dùng.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
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
            })
            .collect()
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
    /// including permissions added later, and cannot be changed or deleted.
    all_permissions: bool,
    /// Permissions the role grants, sorted by name. Empty when `all_permissions`.
    #[schema(example = json!(["auth.roles.read"]))]
    permissions: Vec<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<Role> for RoleResponse {
    fn from(role: Role) -> Self {
        RoleResponse {
            id: role.id,
            name: role.name,
            description: role.description,
            all_permissions: role.all_permissions,
            permissions: role.permissions,
            created_at: role.created_at,
            updated_at: role.updated_at,
        }
    }
}

/// The full content of a role.
#[derive(Debug, Deserialize, Validate, ToSchema)]
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
}

impl From<RoleRequest> for RoleInput {
    fn from(request: RoleRequest) -> Self {
        RoleInput {
            name: request.name,
            description: request.description,
            permissions: request.permissions,
        }
    }
}

/// The complete list of roles a user should have.
#[derive(Debug, Deserialize, Validate, ToSchema)]
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
/// grows when modules are added. Descriptions follow `Accept-Language`. Without
/// `auth.roles.read`, read your own permissions with `GET /api/v1/me/permissions`.
#[utoipa::path(
    get,
    path = "/permissions",
    extensions(("x-permissions" = json!(["auth.roles.read"]))),
    operation_id = "list_permissions",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every permission, sorted by name.", body = Data<Vec<PermissionResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.roles.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_permissions(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    language: autoxide_kernel::i18n::Language,
) -> Result<Data<Vec<PermissionResponse>>, Error> {
    principal.require(&ROLES_READ)?;
    Ok(Data { data: PermissionResponse::catalog(&state.roles, language) })
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
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<RoleResponse>, Error> {
    principal.require(&ROLES_READ)?;
    let page = state.roles.list(&PageRequest::try_from(query)?).await?;
    Ok(page.map(RoleResponse::from))
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
    principal: Principal,
    ValidJson(input): ValidJson<RoleRequest>,
) -> Result<Created<RoleResponse>, Error> {
    principal.require(&ROLES_MANAGE)?;
    let role = state.roles.create(Actor::from(&principal), input.into()).await?;
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
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<RoleResponse>, Error> {
    principal.require(&ROLES_READ)?;
    let role = state.roles.get(id).await?;
    Ok(Tagged { version: role.updated_at, data: role.into() })
}

/// Replace a role: its name, description and permissions.
///
/// Requires `auth.roles.manage`. Users with the role get the new permissions on
/// their next request.
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
        (status = 403, description = "Missing permission `auth.roles.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No role has this id; code `auth.role_not_found`.", body = Problem),
        (status = 409, description = "Another role has this name (`auth.role_name_taken`), or the role is the built-in `admin` (`auth.role_builtin`).", body = Problem),
        (status = 422, description = "A value is invalid, e.g. an unknown permission; see `errors`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
    )
)]
async fn replace_role(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<RoleRequest>,
) -> Result<Tagged<RoleResponse>, Error> {
    principal.require(&ROLES_MANAGE)?;
    let expected = if_match.expected()?;
    let role = state.roles.update(Actor::from(&principal), id, input.into(), expected).await?;
    Ok(Tagged { version: role.updated_at, data: role.into() })
}

/// Delete a role.
///
/// Requires `auth.roles.manage`. Users who had the role lose it.
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
        (status = 403, description = "Missing permission `auth.roles.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No role has this id; code `auth.role_not_found`.", body = Problem),
        (status = 409, description = "The role is the built-in `admin`; code `auth.role_builtin`.", body = Problem),
    )
)]
async fn delete_role(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&ROLES_MANAGE)?;
    state.roles.delete(Actor::from(&principal), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the roles of a user.
///
/// Requires `auth.users.read`.
#[utoipa::path(
    get,
    path = "/users/{id}/roles",
    extensions(("x-permissions" = json!(["auth.users.read"]))),
    operation_id = "get_user_roles",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 200, description = "The user's roles, sorted by name.", body = Data<Vec<RoleResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
    )
)]
async fn get_user_roles(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<RoleResponse>>, Error> {
    principal.require(&USERS_READ)?;
    Ok(roles(state.roles.user_roles(id).await?))
}

/// Replace the roles of a user.
///
/// Requires `auth.roles.manage`. Takes effect on the user's next request. The last
/// user with the `admin` role cannot lose it.
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
        (status = 403, description = "Missing permission `auth.roles.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "No user would keep the `admin` role; code `auth.last_admin`.", body = Problem),
        (status = 422, description = "A role id does not exist, or the list is too long; see `errors`.", body = Problem),
    )
)]
async fn replace_user_roles(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UserRolesRequest>,
) -> Result<Data<Vec<RoleResponse>>, Error> {
    principal.require(&ROLES_MANAGE)?;
    Ok(roles(state.roles.set_user_roles(Actor::from(&principal), id, input.role_ids).await?))
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
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            // Chưa có role nào: đăng nhập được nhưng không làm được gì.
            let (status, _, problem) = call(&app, "GET", "/roles", Some(&ada), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(problem["code"], "permission_denied");
            let (status, _, _) = call(&app, "GET", "/roles", None, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);

            // Cho Ada role `admin` (như lệnh CLI tạo người dùng đầu tiên sẽ làm).
            let page = services.roles.list(&PageRequest::first(10, Order::Asc)).await.unwrap();
            let admin = page.data.iter().find(|role| role.all_permissions).unwrap();
            let user = services.users.find_by_email("ada@example.com").await.unwrap().unwrap();
            services.roles.set_user_roles(Actor::system(), user.id, vec![admin.id]).await.unwrap();

            let (status, _, me) = call(&app, "GET", "/me", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{me}");
            assert_eq!(me["data"]["all_permissions"], true);

            let (status, _, catalog) = call(&app, "GET", "/permissions", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{catalog}");
            assert_eq!(catalog["data"][0]["name"], "auth.passkeys.manage");

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
            let reader = services
                .roles
                .create(
                    Actor::system(),
                    crate::role::RoleInput {
                        name: "reader".to_owned(),
                        description: String::new(),
                        permissions: vec!["auth.roles.read".to_owned()],
                    },
                )
                .await
                .unwrap();
            let user = services.users.find_by_email("ada@example.com").await.unwrap().unwrap();
            services.roles.set_user_roles(Actor::system(), user.id, vec![reader.id]).await.unwrap();
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
}
