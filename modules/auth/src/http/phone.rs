//! Đăng nhập / tạo tài khoản người mua bằng số điện thoại + mã qua SMS (bước 47,
//! D-20261120-phone-accounts). Logic ở `crate::phone`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::HttpState;
use super::session::{MfaChallengeResponse, TokenResponse};
use crate::session::LoginOutcome;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(send_phone_code))
        .routes(routes!(phone_login))
        .routes(routes!(phone_signup))
}

/// A phone number to send a sign-in code to.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhoneCodeRequest {
    /// Mobile number, e.g. `0912345678` or `+84912345678`.
    #[validate(length(min = 1, max = 32))]
    #[schema(example = "0912345678")]
    phone: String,
}

/// A sign-in code was sent.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PhoneCodeResponse {
    /// Send this with the code to `POST /api/v1/auth/phone/login`.
    challenge_id: Uuid,
    /// The code stops working at this time.
    expires_at: Timestamp,
}

/// A sign-in code to check.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhoneLoginRequest {
    /// The `challenge_id` from `POST /api/v1/auth/phone/codes`.
    challenge_id: Uuid,
    /// The 6-digit code from the SMS.
    #[validate(length(min = 1, max = 16))]
    #[schema(example = "123456")]
    code: String,
    /// Display name when this creates a new account (1 to 100 characters); otherwise ignored. Without
    /// it the account is named after the masked number.
    name: Option<String>,
}

/// Send a sign-in code to a phone number.
///
/// Public, for buyers. The answer is the same whether the number already has an account or not. The
/// number must be a mobile number the shop accepts. Codes per number, per address and per hour are
/// limited.
#[utoipa::path(
    post, path = "/phone/codes", extensions(("x-public" = json!(true))),
    operation_id = "send_phone_sign_in_code", request_body = PhoneCodeRequest,
    responses(
        (status = 202, description = "The code was sent.", body = Data<PhoneCodeResponse>),
        (status = 422, description = "Not a mobile number the shop accepts (`compliance.phone_not_allowed`), or a value is invalid.", body = Problem),
        (status = 429, description = "Too many codes; code `compliance.otp_throttled`, see `Retry-After`.", body = Problem),
        (status = 503, description = "This server cannot send SMS (`auth.phone_sign_in_unavailable`, `compliance.sms_disabled`) or the provider did not answer.", body = Problem),
    )
)]
async fn send_phone_code(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    client: ClientInfo,
    ValidJson(input): ValidJson<PhoneCodeRequest>,
) -> Result<Response, Error> {
    let sent = state.phone.send_code(&tenant, &input.phone, client.ip).await?;
    let data = PhoneCodeResponse { challenge_id: sent.id, expires_at: sent.expires_at };
    Ok((StatusCode::ACCEPTED, Data { data }).into_response())
}

/// Sign in, or create a buyer account, with a code sent by SMS.
///
/// Public, for buyers. A number that already has a buyer account signs in to it; a new number
/// creates an account without email or password when the shop lets buyers create accounts (201).
/// When the account has two-step sign-in on, answers 202: continue with
/// `POST /api/v1/auth/login/mfa`. Each code works once and allows a few wrong tries.
#[utoipa::path(
    post, path = "/phone/login", extensions(("x-public" = json!(true))),
    operation_id = "phone_login", request_body = PhoneLoginRequest,
    responses(
        (status = 200, description = "Signed in.", body = Data<TokenResponse>),
        (status = 201, description = "A new buyer account was created and signed in.", body = Data<TokenResponse>),
        (status = 202, description = "Code accepted; the account's second step is needed. Continue with `POST /api/v1/auth/login/mfa`.", body = Data<MfaChallengeResponse>),
        (status = 403, description = "The account is disabled (`auth.user_disabled`), or the shop does not let buyers create accounts (`auth.customer_registration_closed`).", body = Problem),
        (status = 404, description = "No such code; code `compliance.challenge_not_found`.", body = Problem),
        (status = 422, description = "Wrong or expired code (`compliance.otp_invalid`), too many wrong tries (`compliance.otp_attempts_exceeded`), or a value is invalid.", body = Problem),
        (status = 503, description = "This server cannot send SMS; code `auth.phone_sign_in_unavailable`.", body = Problem),
    )
)]
async fn phone_login(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    headers: HeaderMap,
    ValidJson(input): ValidJson<PhoneLoginRequest>,
) -> Result<Response, Error> {
    let user_agent = super::user_agent(&headers);
    let login = state
        .phone
        .login(&tenant, input.challenge_id, &input.code, input.name.as_deref(), user_agent)
        .await?;
    Ok(match login.outcome {
        LoginOutcome::Tokens(tokens) => {
            super::note_country(&state, &tenant, &headers, tokens.session_id).await;
            let status = if login.created { StatusCode::CREATED } else { StatusCode::OK };
            (status, Data { data: TokenResponse::from(tokens) }).into_response()
        }
        LoginOutcome::MfaRequired { mfa_token, expires_in } => {
            let data = MfaChallengeResponse::new(mfa_token, expires_in);
            (StatusCode::ACCEPTED, Data { data }).into_response()
        }
    })
}

/// A verified phone token to create an account from.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhoneSignupRequest {
    /// The order lookup token from `POST /api/v1/orders/public/orders/lookup/{id}/confirm` (the buyer
    /// already proved the number with a one-time code).
    #[validate(length(min = 1, max = 256))]
    lookup_token: String,
    /// Display name for the new account (1 to 100 characters). Without it the account is named after
    /// the masked number.
    name: Option<String>,
}

/// Create a buyer account from a verified order lookup.
///
/// Public, for guest buyers who looked an order up with a code sent by SMS. Creates a buyer account
/// for that phone number, with no email and no password, and signs in. Later sign-ins use a code
/// sent by SMS (`POST /api/v1/auth/phone/codes`) or a passkey added to the account. Next, call
/// `POST /api/v1/orders/me/orders/claim` with the same token to move the guest orders to the
/// account. Limited per address per hour.
#[utoipa::path(
    post, path = "/phone-signups", extensions(("x-public" = json!(true))),
    operation_id = "phone_signup", request_body = PhoneSignupRequest,
    responses(
        (status = 201, description = "A new buyer account was created and signed in.", body = Data<TokenResponse>),
        (status = 403, description = "The shop does not let buyers create accounts; code `auth.customer_registration_closed`.", body = Problem),
        (status = 409, description = "This number already has an account; sign in with a code instead. Code `auth.phone_taken`.", body = Problem),
        (status = 422, description = "The lookup token is not valid or has expired (field `lookup_token`), or a value is invalid.", body = Problem),
        (status = 429, description = "Too many accounts created from this address; code `auth.phone_signup_throttled`, see `Retry-After`.", body = Problem),
        (status = 503, description = "No module issues verified phone tokens on this server; code `auth.phone_signup_unavailable`.", body = Problem),
    )
)]
async fn phone_signup(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    client: ClientInfo,
    headers: HeaderMap,
    ValidJson(input): ValidJson<PhoneSignupRequest>,
) -> Result<Response, Error> {
    let user_agent = super::user_agent(&headers);
    let outcome = state
        .phone
        .sign_up(&tenant, &input.lookup_token, input.name.as_deref(), client.ip, user_agent)
        .await?;
    Ok(match outcome {
        LoginOutcome::Tokens(tokens) => {
            super::note_country(&state, &tenant, &headers, tokens.session_id).await;
            (StatusCode::CREATED, Data { data: TokenResponse::from(tokens) }).into_response()
        }
        // Tài khoản vừa tạo chưa bật hai bước; nhánh này để khớp kiểu.
        LoginOutcome::MfaRequired { mfa_token, expires_in } => {
            let data = MfaChallengeResponse::new(mfa_token, expires_in);
            (StatusCode::ACCEPTED, Data { data }).into_response()
        }
    })
}
