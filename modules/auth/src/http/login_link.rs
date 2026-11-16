//! Đăng nhập bằng link qua email (D-311): `POST /login/link`, `POST /login/link/confirm`.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{OptionalTenant, TenantContext};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use super::HttpState;
use super::session::{MfaChallengeResponse, TokenResponse};
use crate::session::LoginOutcome;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new().routes(routes!(request_link)).routes(routes!(confirm_link))
}

/// Ask for a sign-in link.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginLinkRequest {
    #[validate(length(min = 1, max = 254))]
    #[schema(example = "ada@example.com")]
    email: String,
    /// `staff` (default) or `customer`.
    #[serde(default)]
    realm: autoxide_kernel::access::Realm,
    /// `true`: the link works only together with the `device_secret` returned here, so a link
    /// opened on another device (or intercepted) is useless. Keep the secret on this device.
    #[serde(default)]
    bind_device: bool,
}

/// What to keep while waiting for the mail.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct LoginLinkResponse {
    /// With `bind_device`: send it back with the token. `null` otherwise. Returned even when no
    /// account has this email, so the answer reveals nothing.
    device_secret: Option<String>,
}

/// Finish signing in with the token from the mail.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LoginLinkConfirmation {
    #[validate(length(min = 1, max = 512))]
    token: String,
    /// The `device_secret` from `POST /api/v1/auth/login/link`, when it asked to bind the device.
    #[validate(length(max = 256))]
    device_secret: Option<String>,
}

/// Send a sign-in link by email.
///
/// No sign-in needed; off unless the server turns it on (`AUTOXIDE_AUTH_LOGIN_LINK`). Always
/// answers 202 the same way, whether or not an account has this email; the mail (if any) goes
/// out shortly after. The link works once, for `AUTOXIDE_AUTH_LOGIN_LINK_MINUTES` minutes.
#[utoipa::path(
    post,
    path = "/login/link",
    extensions(("x-public" = json!(true))),
    operation_id = "request_login_link",
    request_body = LoginLinkRequest,
    responses(
        (status = 202, description = "If the account exists, a link is on its way.", body = Data<LoginLinkResponse>),
        (status = 404, description = "Sign-in links are off on this server; code `auth.login_link_disabled`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn request_link(
    State(state): State<Arc<HttpState>>,
    tenant: TenantContext,
    language: Language,
    ValidJson(input): ValidJson<LoginLinkRequest>,
) -> Result<(StatusCode, Data<LoginLinkResponse>), Error> {
    let device_secret = state
        .login_links
        .request(&tenant, &input.email, input.realm, input.bind_device, language)
        .await?;
    Ok((StatusCode::ACCEPTED, Data { data: LoginLinkResponse { device_secret } }))
}

/// Sign in with the token from a sign-in link.
///
/// The token works once. When two-step sign-in is on, answers 202 with an `mfa_token`, as
/// `POST /api/v1/auth/login` does. Needs no tenant header: the token names its tenant.
#[utoipa::path(
    post,
    path = "/login/link/confirm",
    extensions(("x-public" = json!(true))),
    operation_id = "confirm_login_link",
    request_body = LoginLinkConfirmation,
    responses(
        (status = 200, description = "Signed in.", body = Data<TokenResponse>),
        (status = 202, description = "A code is needed; continue with `POST /api/v1/auth/login/mfa`.", body = Data<MfaChallengeResponse>),
        (status = 401, description = "The token is wrong, used or expired, or the device secret does not match; code `auth.login_link_invalid`.", body = Problem),
        (status = 403, description = "The account is disabled; code `auth.user_disabled`.", body = Problem),
        (status = 404, description = "Sign-in links are off on this server; code `auth.login_link_disabled`.", body = Problem),
        (status = 422, description = "The body is missing a field or a value is invalid.", body = Problem),
    )
)]
async fn confirm_link(
    State(state): State<Arc<HttpState>>,
    OptionalTenant(tenant): OptionalTenant,
    headers: HeaderMap,
    ValidJson(input): ValidJson<LoginLinkConfirmation>,
) -> Result<Response, Error> {
    let user_agent = super::user_agent(&headers);
    let (tenant, outcome) = state
        .login_links
        .confirm(tenant.as_ref(), &input.token, input.device_secret.as_deref(), user_agent)
        .await?;
    Ok(match outcome {
        LoginOutcome::Tokens(tokens) => {
            super::note_country(&state, &tenant, &headers, tokens.session_id).await;
            Data { data: TokenResponse::from(tokens) }.into_response()
        }
        LoginOutcome::MfaRequired { mfa_token, expires_in } => {
            let data = MfaChallengeResponse::new(mfa_token, expires_in);
            (StatusCode::ACCEPTED, Data { data }).into_response()
        }
    })
}
