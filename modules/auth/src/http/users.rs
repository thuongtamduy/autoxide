//! Endpoint người dùng: quản trị viên tạo và quản lý tài khoản; người lạ tự đăng ký
//! khi được bật.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
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
use crate::api::User;
use crate::permissions::{USERS_MANAGE, USERS_READ};
use crate::user::{NewUser, UserChanges};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(list_users, create_user))
        .routes(routes!(get_user, update_user))
        .routes(routes!(register))
}

/// A user account. Never includes the password.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct UserResponse {
    id: Uuid,
    /// Normalized email: lower case, no surrounding spaces. Unique.
    #[schema(example = "ada@example.com")]
    email: String,
    /// Display name.
    #[schema(example = "Ada Lovelace")]
    name: String,
    /// When the account was disabled; `null` while it is active. A disabled account
    /// cannot sign in or use its tokens.
    disabled_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        UserResponse {
            id: user.id,
            email: user.email,
            name: user.name,
            disabled_at: user.disabled_at,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// A new account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct CreateUserRequest {
    /// Email; also the sign-in name. Case and surrounding spaces do not matter.
    #[validate(length(min = 1, max = 254))]
    #[schema(example = "ada@example.com")]
    email: String,
    /// Display name, 1 to 200 characters.
    #[validate(length(min = 1, max = 200))]
    #[schema(example = "Ada Lovelace")]
    name: String,
    /// 12 to 128 characters. When omitted, the account cannot sign in with a password
    /// until one is set.
    #[validate(length(max = 128))]
    password: Option<String>,
}

/// A new account created by its owner.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct RegisterRequest {
    /// Email; also the sign-in name.
    #[validate(length(min = 1, max = 254))]
    #[schema(example = "ada@example.com")]
    email: String,
    /// Display name, 1 to 200 characters.
    #[validate(length(min = 1, max = 200))]
    #[schema(example = "Ada Lovelace")]
    name: String,
    /// 12 to 128 characters.
    #[validate(length(min = 1, max = 128))]
    password: String,
}

/// Changes to an account. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct UpdateUserRequest {
    /// New display name, 1 to 200 characters.
    #[validate(length(min = 1, max = 200))]
    name: Option<String>,
    /// `true` disables the account and signs it out everywhere; `false` enables it.
    disabled: Option<bool>,
}

/// List user accounts.
///
/// Requires `auth.users.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/users",
    operation_id = "list_users",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of users.", body = Page<UserResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_users(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<UserResponse>, Error> {
    principal.require(&USERS_READ)?;
    let page = state.users.list(&PageRequest::try_from(query)?).await?;
    Ok(page.map(UserResponse::from))
}

/// Create a user account.
///
/// Requires `auth.users.manage`. The account has no role; give it roles with
/// `PUT /api/v1/auth/users/{id}/roles`.
#[utoipa::path(
    post,
    path = "/users",
    operation_id = "create_user",
    security(("bearer" = [])),
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "The account was created.", body = Data<UserResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "An account has this email; code `auth.email_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_user(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    ValidJson(input): ValidJson<CreateUserRequest>,
) -> Result<(StatusCode, Data<UserResponse>), Error> {
    principal.require(&USERS_MANAGE)?;
    let input = NewUser { email: input.email, name: input.name, password: input.password };
    let user = state.users.create(input).await?;
    Ok((StatusCode::CREATED, Data { data: user.into() }))
}

/// Get a user account.
///
/// Requires `auth.users.read`.
#[utoipa::path(
    get,
    path = "/users/{id}",
    operation_id = "get_user",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 200, description = "The account.", body = Data<UserResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
    )
)]
async fn get_user(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<UserResponse>, Error> {
    principal.require(&USERS_READ)?;
    let user = state.users.get(id).await?.ok_or_else(|| crate::user::user_not_found(id))?;
    Ok(Data { data: user.into() })
}

/// Rename, disable or enable a user account.
///
/// Requires `auth.users.manage`. Disabling signs the user out everywhere: their
/// access tokens stop working at once. The last active user with the `admin` role
/// cannot be disabled.
#[utoipa::path(
    patch,
    path = "/users/{id}",
    operation_id = "update_user",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "The account after the change.", body = Data<UserResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
        (status = 409, description = "No active user would keep the `admin` role; code `auth.last_admin`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_user(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UpdateUserRequest>,
) -> Result<Data<UserResponse>, Error> {
    principal.require(&USERS_MANAGE)?;
    let changes = UserChanges { name: input.name, disabled: input.disabled };
    Ok(Data { data: state.users.update(id, changes).await?.into() })
}

/// Create your own account.
///
/// Open only when the server enables self-registration; otherwise answers 403. The
/// new account has no role. Then sign in with `POST /api/v1/auth/login`.
#[utoipa::path(
    post,
    path = "/register",
    operation_id = "register",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "The account was created.", body = Data<UserResponse>),
        (status = 403, description = "Self-registration is turned off; code `auth.registration_disabled`.", body = Problem),
        (status = 409, description = "An account has this email; code `auth.email_taken`.", body = Problem),
        (status = 422, description = "A value is invalid, e.g. a password shorter than 12 characters; see `errors`.", body = Problem),
    )
)]
async fn register(
    State(state): State<Arc<HttpState>>,
    ValidJson(input): ValidJson<RegisterRequest>,
) -> Result<(StatusCode, Data<UserResponse>), Error> {
    if !state.self_registration {
        return Err(Error::forbidden(
            "auth.registration_disabled",
            "This server does not let people create their own accounts. Ask an administrator \
             to create one for you.",
        ));
    }
    let input = NewUser { email: input.email, name: input.name, password: Some(input.password) };
    let user = state.users.create(input).await?;
    Ok((StatusCode::CREATED, Data { data: user.into() }))
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::api::{Accounts, NewAccount};
    use crate::http::test_support::{PASSWORD, app, app_with, bearer, call};

    #[tokio::test]
    async fn quan_tri_vien_tao_xem_va_khoa_nguoi_dung() {
        TestDatabase::run(async |test| {
            let (app, services) = app(test).await;
            let root = NewAccount {
                email: "root@example.com".to_owned(),
                name: "Root".to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin: true,
            };
            Accounts::create(services.users.as_ref(), root).await.unwrap();
            let admin = bearer(&app, "root@example.com", PASSWORD).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            // Người không có quyền không tạo được.
            let body = json!({"email": "bob@example.com", "name": "Bob", "password": PASSWORD});
            let (status, _, _) = call(&app, "POST", "/users", Some(&ada), Some(body.clone())).await;
            assert_eq!(status, StatusCode::FORBIDDEN);

            let (status, _, created) = call(&app, "POST", "/users", Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            assert_eq!(created["data"]["email"], "bob@example.com");
            assert!(created["data"]["disabled_at"].is_null());
            assert!(created["data"].get("password").is_none());
            let id = created["data"]["id"].as_str().unwrap().to_owned();

            let (status, _, list) = call(&app, "GET", "/users?limit=2", Some(&admin), None).await;
            assert_eq!(status, StatusCode::OK, "{list}");
            assert_eq!(list["data"][0]["email"], "bob@example.com");

            let path = format!("/users/{id}");
            let (status, _, got) = call(&app, "GET", &path, Some(&admin), None).await;
            assert_eq!((status, got["data"]["name"].clone()), (StatusCode::OK, json!("Bob")));

            let bob = bearer(&app, "bob@example.com", PASSWORD).await;
            let body = json!({"disabled": true});
            let (status, _, changed) = call(&app, "PATCH", &path, Some(&admin), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{changed}");
            assert!(changed["data"]["disabled_at"].is_string());
            // Access token của người bị khoá mất hiệu lực ngay.
            let (status, _, problem) = call(&app, "GET", "/me", Some(&bob), None).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(problem["code"], "auth.user_disabled");

            let missing = format!("/users/{}", uuid::Uuid::now_v7());
            let (status, _, problem) = call(&app, "GET", &missing, Some(&admin), None).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(problem["code"], "auth.user_not_found");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tu_dang_ky_mac_dinh_tat() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let body = json!({"email": "eve@example.com", "name": "Eve", "password": PASSWORD});

            let (status, _, problem) = call(&app, "POST", "/register", None, Some(body)).await;

            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(problem["code"], "auth.registration_disabled");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tu_dang_ky_khi_bat_roi_dang_nhap_duoc() {
        TestDatabase::run(async |test| {
            let (app, _) = app_with(test, true).await;
            let body = json!({"email": "Eve@Example.com", "name": "Eve", "password": PASSWORD});

            let (status, _, created) =
                call(&app, "POST", "/register", None, Some(body.clone())).await;
            assert_eq!(status, StatusCode::CREATED, "{created}");
            assert_eq!(created["data"]["email"], "eve@example.com");

            let eve = bearer(&app, "eve@example.com", PASSWORD).await;
            let (_, _, me) = call(&app, "GET", "/me", Some(&eve), None).await;
            assert_eq!(me["data"]["permissions"], json!([]));
            assert_eq!(me["data"]["all_permissions"], false);

            let (status, _, taken) = call(&app, "POST", "/register", None, Some(body)).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(taken["code"], "auth.email_taken");
            let short = json!({"email": "x@example.com", "name": "X", "password": "short"});
            let (status, _, _) = call(&app, "POST", "/register", None, Some(short)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        })
        .await
        .unwrap();
    }
}
