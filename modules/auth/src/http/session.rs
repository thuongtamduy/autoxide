//! Endpoint đăng nhập: `/login`, `/refresh`, `/logout`, `/me`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use axum::extract::State;
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
use crate::session::Tokens;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(refresh))
        .routes(routes!(logout))
        .routes(routes!(me))
}

/// Email and password of an account.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct LoginRequest {
    /// Email of the account. Case and surrounding spaces do not matter.
    #[validate(length(min = 1, max = 254))]
    #[schema(example = "ada@example.com")]
    email: String,
    #[validate(length(min = 1, max = 1024))]
    #[schema(example = "correct horse battery staple")]
    password: String,
}

/// A refresh token, as returned by sign-in or by the previous refresh.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct RefreshRequest {
    #[validate(length(min = 1, max = 256))]
    refresh_token: String,
}

/// A new pair of tokens.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct TokenResponse {
    /// Send this with every request: `Authorization: Bearer <access_token>`.
    access_token: String,
    /// Always `Bearer`.
    #[schema(example = "Bearer")]
    token_type: String,
    /// Seconds until `access_token` expires. Refresh shortly before then.
    #[schema(example = 900)]
    expires_in: u64,
    /// Exchange this for a new pair at `POST /api/v1/auth/refresh`. Each refresh
    /// token works once: always keep only the newest one. Store it securely.
    refresh_token: String,
    /// When `refresh_token` expires; after that, sign in again.
    refresh_token_expires_at: DateTime<Utc>,
}

impl From<Tokens> for TokenResponse {
    fn from(tokens: Tokens) -> Self {
        TokenResponse {
            access_token: tokens.access_token,
            token_type: "Bearer".to_owned(),
            expires_in: tokens.expires_in,
            refresh_token: tokens.refresh_token,
            refresh_token_expires_at: tokens.refresh_token_expires_at,
        }
    }
}

/// The signed-in user and what they may do.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MeResponse {
    id: Uuid,
    /// Normalized email: lower case, no surrounding spaces.
    #[schema(example = "ada@example.com")]
    email: String,
    /// Display name.
    #[schema(example = "Ada Lovelace")]
    name: String,
    created_at: DateTime<Utc>,
    /// `true` when a role grants every permission (the `admin` role); `permissions`
    /// is then empty.
    all_permissions: bool,
    /// Permissions granted by the user's roles, sorted by name.
    #[schema(example = json!(["auth.roles.read"]))]
    permissions: Vec<String>,
}

impl MeResponse {
    fn new(user: User, principal: &Principal) -> Self {
        MeResponse {
            id: user.id,
            email: user.email,
            name: user.name,
            created_at: user.created_at,
            all_permissions: principal.has_all_permissions(),
            permissions: principal.permissions(),
        }
    }
}

/// Sign in with email and password.
///
/// Returns a short-lived access token and a refresh token. The same error answers
/// an unknown email and a wrong password.
#[utoipa::path(
    post,
    path = "/login",
    operation_id = "login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in.", body = Data<TokenResponse>),
        (status = 401, description = "Wrong email or password; code `auth.invalid_credentials`.", body = Problem),
        (status = 403, description = "The account is disabled; code `auth.user_disabled`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn login(
    State(state): State<Arc<HttpState>>,
    ValidJson(input): ValidJson<LoginRequest>,
) -> Result<Data<TokenResponse>, Error> {
    let tokens = state.sessions.login(&input.email, &input.password).await?;
    Ok(Data { data: tokens.into() })
}

/// Exchange a refresh token for a new pair of tokens.
///
/// The refresh token sent is used up. Sending a used refresh token again ends the
/// whole session (it may have been stolen): sign in again.
#[utoipa::path(
    post,
    path = "/refresh",
    operation_id = "refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "A new pair of tokens.", body = Data<TokenResponse>),
        (status = 401, description = "The refresh token is unknown, expired or revoked (code `auth.invalid_refresh_token`), or was already used (code `auth.refresh_token_reused`); sign in again.", body = Problem),
        (status = 403, description = "The account is disabled; code `auth.user_disabled`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn refresh(
    State(state): State<Arc<HttpState>>,
    ValidJson(input): ValidJson<RefreshRequest>,
) -> Result<Data<TokenResponse>, Error> {
    let tokens = state.sessions.refresh(&input.refresh_token).await?;
    Ok(Data { data: tokens.into() })
}

/// Sign out: end the session of a refresh token.
///
/// Every refresh token of the session stops working; access tokens already issued
/// keep working until they expire. Safe to repeat.
#[utoipa::path(
    post,
    path = "/logout",
    operation_id = "logout",
    request_body = RefreshRequest,
    responses(
        (status = 204, description = "Signed out (also when the token was unknown or already revoked)."),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn logout(
    State(state): State<Arc<HttpState>>,
    ValidJson(input): ValidJson<RefreshRequest>,
) -> Result<StatusCode, Error> {
    state.sessions.logout(&input.refresh_token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Get the signed-in user and their permissions.
#[utoipa::path(
    get,
    path = "/me",
    operation_id = "me",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The user the access token belongs to, with the permissions of their roles.", body = Data<MeResponse>),
        (status = 401, description = "The access token is missing (code `auth.missing_token`), invalid (`auth.invalid_token`) or expired (`auth.token_expired`).", body = Problem),
        (status = 403, description = "The account is disabled; code `auth.user_disabled`.", body = Problem),
    )
)]
async fn me(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
) -> Result<Data<MeResponse>, Error> {
    let user =
        state.users.get(principal.user_id()).await?.ok_or_else(crate::token::invalid_token)?;
    Ok(Data { data: MeResponse::new(user, &principal) })
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::{StatusCode, header};
    use serde_json::{Value, json};

    use crate::http::test_support::{PASSWORD, app, call};

    #[tokio::test]
    async fn dang_nhap_xem_minh_doi_token_dang_xuat() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let credentials = json!({"email": "ada@example.com", "password": PASSWORD});

            let (status, _, login) = call(&app, "POST", "/login", None, Some(credentials)).await;
            assert_eq!(status, StatusCode::OK, "{login}");
            assert_eq!(login["data"]["token_type"], "Bearer");
            assert_eq!(login["data"]["expires_in"], 900);
            let access = login["data"]["access_token"].as_str().unwrap();
            let refresh = login["data"]["refresh_token"].as_str().unwrap();

            let bearer = format!("Bearer {access}");
            let (status, _, me) = call(&app, "GET", "/me", Some(&bearer), None).await;
            assert_eq!(status, StatusCode::OK, "{me}");
            assert_eq!(me["data"]["email"], "ada@example.com");
            assert!(me["data"].get("password_hash").is_none());

            let body = json!({"refresh_token": refresh});
            let (status, _, renewed) = call(&app, "POST", "/refresh", None, Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{renewed}");
            let renewed_token = renewed["data"]["refresh_token"].as_str().unwrap();

            let body = json!({"refresh_token": renewed_token});
            let (status, _, empty) = call(&app, "POST", "/logout", None, Some(body.clone())).await;
            assert_eq!(status, StatusCode::NO_CONTENT);
            assert_eq!(empty, Value::Null);
            let (status, _, problem) = call(&app, "POST", "/refresh", None, Some(body)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(problem["code"], "auth.invalid_refresh_token");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn loi_xac_thuc_la_problem_kem_www_authenticate() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let wrong = json!({"email": "ada@example.com", "password": "wrong password"});

            let (status, headers, problem) = call(&app, "POST", "/login", None, Some(wrong)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
            assert_eq!(problem["code"], "auth.invalid_credentials");

            let (status, _, problem) = call(&app, "GET", "/me", None, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(problem["code"], "auth.missing_token");

            let empty = json!({"email": "", "password": ""});
            let (status, _, problem) = call(&app, "POST", "/login", None, Some(empty)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(problem["code"], "validation_failed");
        })
        .await
        .unwrap();
    }
}
