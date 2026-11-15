//! Endpoint đăng nhập: `/login`, `/refresh`, `/logout`, `/me`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use crate::api::User;
use crate::session::{LoginOutcome, Tokens};

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(login))
        .routes(routes!(login_mfa))
        .routes(routes!(refresh))
        .routes(routes!(logout))
        .routes(routes!(me))
        .routes(routes!(change_password))
}

/// The current password and the new one.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct ChangePasswordRequest {
    current_password: String,
    /// 12 to 128 characters.
    new_password: String,
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

/// Two-step sign-in: the password was right, a code is needed.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MfaChallengeResponse {
    /// Send this with the code to `POST /api/v1/auth/login/mfa`. Works once.
    mfa_token: String,
    /// Seconds left to send the code.
    #[schema(example = 300)]
    expires_in: u64,
    /// Accepted kinds of code: `totp` (6 digits from the authenticator app) and
    /// `recovery_code`.
    #[schema(example = json!(["totp", "recovery_code"]))]
    methods: Vec<String>,
}

/// The second step of signing in.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct LoginMfaRequest {
    /// The `mfa_token` returned by `POST /api/v1/auth/login`.
    #[validate(length(min = 1, max = 256))]
    mfa_token: String,
    /// The current 6-digit code from the authenticator app, or an unused recovery code.
    #[validate(length(min = 1, max = 64))]
    #[schema(example = "123456")]
    code: String,
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
    /// The session these tokens belong to. It stays the same across refreshes; keep it
    /// to recognize this client in `GET /api/v1/auth/me/sessions`.
    session_id: Uuid,
}

impl From<Tokens> for TokenResponse {
    fn from(tokens: Tokens) -> Self {
        TokenResponse {
            access_token: tokens.access_token,
            token_type: "Bearer".to_owned(),
            expires_in: tokens.expires_in,
            refresh_token: tokens.refresh_token,
            refresh_token_expires_at: tokens.refresh_token_expires_at,
            session_id: tokens.session_id,
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
    /// When the email was confirmed through a verification mail; `null` while it is
    /// not verified (ask for a new mail with `POST /api/v1/auth/me/email/verification`).
    email_verified_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    /// `true` when a role grants every permission (the `admin` role); `permissions`
    /// is then empty.
    all_permissions: bool,
    /// Permissions granted by the user's roles, sorted by name.
    #[schema(example = json!(["auth.roles.read"]))]
    permissions: Vec<String>,
    /// Whether the account has a password. Without one (signed in with a passkey), set a first
    /// password through `POST /api/v1/auth/password-reset`.
    has_password: bool,
    /// The new address of an email change waiting to be confirmed; `null` when there is none.
    /// Cancel it with `DELETE /api/v1/auth/me/email`.
    pending_email: Option<PendingEmail>,
}

/// An email change waiting for its confirmation link.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PendingEmail {
    #[schema(example = "ada@new.example")]
    email: String,
    /// When the confirmation link stops working.
    expires_at: DateTime<Utc>,
}

impl MeResponse {
    /// Tài khoản của người gọi, kèm có mật khẩu chưa và email đang chờ xác minh.
    pub(crate) async fn load(
        state: &HttpState,
        user: User,
        principal: &Principal,
    ) -> Result<Self, Error> {
        let facts = state.users.facts(&[user.id]).await?.remove(&user.id).unwrap_or_default();
        let pending = state.verifications.pending_change(&user).await?;
        Ok(MeResponse {
            id: user.id,
            email: user.email,
            name: user.name,
            email_verified_at: user.email_verified_at,
            created_at: user.created_at,
            all_permissions: principal.has_all_permissions(),
            permissions: principal.permissions(),
            has_password: facts.has_password,
            pending_email: pending.map(|(email, expires_at)| PendingEmail { email, expires_at }),
        })
    }
}

/// Sign in with email and password.
///
/// Returns a short-lived access token and a refresh token. When the account has
/// two-step sign-in on, answers 202 instead: send the `mfa_token` and a code to
/// `POST /api/v1/auth/login/mfa`. The same error answers an unknown email and a
/// wrong password. After several failed attempts the email is locked for a while,
/// even for the right password.
#[utoipa::path(
    post,
    path = "/login",
    extensions(("x-public" = json!(true))),
    operation_id = "login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Signed in.", body = Data<TokenResponse>),
        (status = 202, description = "Password accepted; a code is needed. Continue with `POST /api/v1/auth/login/mfa`.", body = Data<MfaChallengeResponse>),
        (status = 401, description = "Wrong email or password; code `auth.invalid_credentials`.", body = Problem),
        (status = 403, description = "The account is disabled; code `auth.user_disabled`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
        (status = 429, description = "Too many failed sign-ins for this email; code `auth.login_throttled`. Wait the number of seconds in the `Retry-After` header.", body = Problem),
    )
)]
async fn login(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    ValidJson(input): ValidJson<LoginRequest>,
) -> Result<Response, Error> {
    let user_agent = super::user_agent(&headers);
    // Hai kết quả, hai hình dạng: `Response` là kiểu chung của mọi response.
    Ok(match state.sessions.login(&input.email, &input.password, user_agent).await? {
        LoginOutcome::Tokens(tokens) => {
            super::note_country(&state, &headers, tokens.session_id).await;
            Data { data: TokenResponse::from(tokens) }.into_response()
        }
        LoginOutcome::MfaRequired { mfa_token, expires_in } => {
            let data = MfaChallengeResponse {
                mfa_token,
                expires_in,
                methods: METHODS.map(String::from).to_vec(),
            };
            (StatusCode::ACCEPTED, Data { data }).into_response()
        }
    })
}

/// Cách nhập lớp thứ hai mà `/login/mfa` nhận.
const METHODS: [&str; 2] = ["totp", "recovery_code"];

/// Finish signing in with a code from the authenticator app or a recovery code.
///
/// Send the `mfa_token` from `POST /api/v1/auth/login` within its `expires_in`
/// seconds. Each `mfa_token` allows five wrong codes; wrong codes also count
/// towards the sign-in lock of the email.
#[utoipa::path(
    post,
    path = "/login/mfa",
    extensions(("x-public" = json!(true))),
    operation_id = "login_mfa",
    request_body = LoginMfaRequest,
    responses(
        (status = 200, description = "Signed in.", body = Data<TokenResponse>),
        (status = 401, description = "The code is wrong (`auth.invalid_mfa_code`), or the `mfa_token` is unknown, expired or used up (`auth.invalid_mfa_token`: sign in again).", body = Problem),
        (status = 403, description = "The account is disabled; code `auth.user_disabled`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
        (status = 429, description = "Too many failed attempts for this email; code `auth.login_throttled`. Wait the number of seconds in the `Retry-After` header.", body = Problem),
    )
)]
async fn login_mfa(
    State(state): State<Arc<HttpState>>,
    headers: HeaderMap,
    ValidJson(input): ValidJson<LoginMfaRequest>,
) -> Result<Data<TokenResponse>, Error> {
    let user_agent = super::user_agent(&headers);
    let tokens = state.sessions.complete_mfa(&input.mfa_token, &input.code, user_agent).await?;
    super::note_country(&state, &headers, tokens.session_id).await;
    Ok(Data { data: tokens.into() })
}

/// Exchange a refresh token for a new pair of tokens.
///
/// The refresh token sent is used up. Sending a used refresh token again ends the
/// whole session (it may have been stolen): sign in again.
#[utoipa::path(
    post,
    path = "/refresh",
    extensions(("x-public" = json!(true))),
    operation_id = "refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "A new pair of tokens.", body = Data<TokenResponse>),
        (status = 401, description = "The refresh token is unknown, expired or revoked (code `auth.invalid_refresh_token`), or was already used (code `auth.refresh_token_reused`); sign in again with `POST /api/v1/auth/login`.", body = Problem),
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
    extensions(("x-public" = json!(true))),
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
    extensions(("x-permissions" = json!([]))),
    operation_id = "me",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The user the access token belongs to, with the permissions of their roles.", body = Data<MeResponse>),
        (status = 401, description = "The access token is missing (code `auth.missing_token`), invalid (`auth.invalid_token`) or expired (`auth.token_expired`; get a new one with `POST /api/v1/auth/refresh`). Without a usable refresh token, sign in with `POST /api/v1/auth/login`.", body = Problem),
        (status = 403, description = "The account is disabled; code `auth.user_disabled`.", body = Problem),
    )
)]
async fn me(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
) -> Result<Data<MeResponse>, Error> {
    let user =
        state.users.get(principal.user_id()).await?.ok_or_else(crate::token::invalid_token)?;
    Ok(Data { data: MeResponse::load(&state, user, &principal).await? })
}

/// Change your own password.
///
/// Needs the current password and a session signed in with a password (an API key
/// cannot do this). Every session of the account ends, including this one; the
/// response carries the tokens of a new session, so this client stays signed in.
/// Wrong current passwords count towards the sign-in lock of the email.
#[utoipa::path(
    post,
    path = "/me/password",
    extensions(("x-permissions" = json!([]))),
    operation_id = "change_password",
    security(("bearer" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 200, description = "Changed; use these tokens from now on.", body = Data<TokenResponse>),
        (status = 400, description = "The current password is wrong; code `auth.wrong_password`. A user who forgot it resets it with `POST /api/v1/auth/password-reset`.", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Called with an API key (code `auth.session_required`), or the account is disabled (code `auth.user_disabled`).", body = Problem),
        (status = 422, description = "The body is missing a field, or the new password breaks the rules (see `errors`).", body = Problem),
        (status = 429, description = "Too many wrong passwords for this email; code `auth.login_throttled`. Wait the number of seconds in the `Retry-After` header.", body = Problem),
    )
)]
async fn change_password(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    headers: HeaderMap,
    ValidJson(input): ValidJson<ChangePasswordRequest>,
) -> Result<Data<TokenResponse>, Error> {
    super::require_session(&principal)?;
    let user_agent = super::user_agent(&headers);
    let tokens = state
        .sessions
        .change_password(
            principal.user_id(),
            &input.current_password,
            input.new_password,
            user_agent,
        )
        .await?;
    super::note_country(&state, &headers, tokens.session_id).await;
    Ok(Data { data: tokens.into() })
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

            // Cùng lỗi, bằng tiếng Việt khi client yêu cầu.
            let response = app
                .post("/api/v1/auth/login")
                .header(header::ACCEPT_LANGUAGE.as_str(), "vi")
                .json(json!({"email": "ada@example.com", "password": "wrong password"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.headers()[header::CONTENT_LANGUAGE], "vi");
            let vi: Value = serde_json::from_slice(response.body()).unwrap();
            assert_eq!(vi["code"], "auth.invalid_credentials");
            assert_eq!(vi["detail"], "Email hoặc mật khẩu không đúng.");

            let empty = json!({"email": "", "password": ""});
            let (status, _, problem) = call(&app, "POST", "/login", None, Some(empty)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(problem["code"], "validation_failed");
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tu_doi_mat_khau_qua_api() {
        TestDatabase::run(async |test| {
            let (app, _) = crate::http::test_support::app(test).await;
            let ada = crate::http::test_support::bearer(&app, "ada@example.com", PASSWORD).await;
            let new = "a brand new passphrase";

            let body = json!({"current_password": "wrong one", "new_password": new});
            let (status, _, problem) =
                call(&app, "POST", "/me/password", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(problem["code"], "auth.wrong_password");

            let body = json!({"current_password": PASSWORD, "new_password": new});
            let (status, _, changed) =
                call(&app, "POST", "/me/password", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{changed}");
            assert!(changed["data"]["refresh_token"].is_string());
            crate::http::test_support::bearer(&app, "ada@example.com", new).await;
        })
        .await
        .unwrap();
    }
}
