//! Endpoint MFA: người dùng bật, xác nhận, tắt TOTP của chính mình; quản trị viên
//! xoá MFA của người dùng mất thiết bị.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use super::{HttpState, require_session};
use crate::mfa::MfaStatus;
use crate::permissions::USERS_MANAGE;

pub(super) fn routes() -> OpenApiRouter<Arc<HttpState>> {
    OpenApiRouter::new()
        .routes(routes!(mfa_status))
        .routes(routes!(start_totp))
        .routes(routes!(confirm_totp))
        .routes(routes!(disable_totp))
        .routes(routes!(regenerate_recovery_codes))
        .routes(routes!(reset_user_mfa))
}

/// Your two-step sign-in settings.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MfaStatusResponse {
    /// Sign-in asks for a code from an authenticator app.
    totp_enabled: bool,
    /// Unused recovery codes left. Make new ones before they run out.
    recovery_codes_remaining: u64,
}

impl From<MfaStatus> for MfaStatusResponse {
    fn from(status: MfaStatus) -> Self {
        MfaStatusResponse {
            totp_enabled: status.totp_enabled,
            recovery_codes_remaining: status.recovery_codes_remaining,
        }
    }
}

/// One-time recovery codes. Shown only once.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct RecoveryCodesResponse {
    /// Each code signs in once in place of an authenticator code, e.g. after losing
    /// the phone. Store them somewhere safe, away from the phone.
    #[schema(example = json!(["abcd-efgh-ijkl-mnop", "qrst-uvwx-yz23-4567"]))]
    recovery_codes: Vec<String>,
}

/// What to enter in the authenticator app.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct TotpSetupResponse {
    /// The secret, base32, for typing by hand.
    #[schema(example = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP")]
    secret: String,
    /// The same secret as an `otpauth://` URI: show it as a QR code to scan.
    #[schema(
        example = "otpauth://totp/Autoxide:ada%40example.com?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Autoxide"
    )]
    otpauth_url: String,
}

/// A code from the authenticator app, or (where stated) a recovery code.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct CodeRequest {
    /// The 6-digit code currently shown by the app, or a recovery code where the
    /// endpoint accepts one.
    #[validate(length(min = 1, max = 64))]
    #[schema(example = "123456")]
    code: String,
}

/// Get your two-step sign-in settings.
#[utoipa::path(
    get,
    path = "/me/mfa",
    operation_id = "get_mfa",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Your settings.", body = Data<MfaStatusResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn mfa_status(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
) -> Result<Data<MfaStatusResponse>, Error> {
    Ok(Data { data: state.mfa.status(principal.user_id()).await?.into() })
}

/// Start turning on two-step sign-in with an authenticator app.
///
/// Returns a new secret to add to the app. Nothing changes until you confirm with
/// a code (`POST /api/v1/auth/me/mfa/totp/confirm`). Calling again replaces an
/// unconfirmed secret. Needs a password session, not an API key.
#[utoipa::path(
    post,
    path = "/me/mfa/totp",
    operation_id = "start_totp",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Add this secret to the authenticator app, then confirm.", body = Data<TotpSetupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`.", body = Problem),
        (status = 409, description = "Two-step sign-in is already on; code `auth.mfa_already_enabled`.", body = Problem),
    )
)]
async fn start_totp(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
) -> Result<Data<TotpSetupResponse>, Error> {
    require_session(&principal)?;
    let user =
        state.users.get(principal.user_id()).await?.ok_or_else(crate::token::invalid_token)?;
    let setup = state.mfa.start_totp(user.id, &user.email).await?;
    Ok(Data { data: TotpSetupResponse { secret: setup.secret, otpauth_url: setup.otpauth_url } })
}

/// Confirm two-step sign-in with the first code from the app.
///
/// From now on, signing in asks for a code. The response holds ten one-time
/// recovery codes, shown only this once. Needs a password session.
#[utoipa::path(
    post,
    path = "/me/mfa/totp/confirm",
    operation_id = "confirm_totp",
    security(("bearer" = [])),
    request_body = CodeRequest,
    responses(
        (status = 200, description = "Two-step sign-in is on; store the recovery codes now.", body = Data<RecoveryCodesResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`.", body = Problem),
        (status = 409, description = "No setup was started (`auth.mfa_setup_not_started`), or it is already on (`auth.mfa_already_enabled`).", body = Problem),
        (status = 422, description = "The code is wrong or expired; `errors` names the field `code`.", body = Problem),
    )
)]
async fn confirm_totp(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    ValidJson(input): ValidJson<CodeRequest>,
) -> Result<Data<RecoveryCodesResponse>, Error> {
    require_session(&principal)?;
    let recovery_codes = state.mfa.confirm_totp(principal.user_id(), &input.code).await?;
    Ok(Data { data: RecoveryCodesResponse { recovery_codes } })
}

/// Replace your recovery codes with ten new ones.
///
/// The old codes stop working. Needs a current authenticator code (not a recovery
/// code) and a password session.
#[utoipa::path(
    post,
    path = "/me/mfa/recovery-codes",
    operation_id = "regenerate_recovery_codes",
    security(("bearer" = [])),
    request_body = CodeRequest,
    responses(
        (status = 200, description = "The new codes; store them now.", body = Data<RecoveryCodesResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`.", body = Problem),
        (status = 409, description = "Two-step sign-in is not on; code `auth.mfa_not_enabled`.", body = Problem),
        (status = 422, description = "The code is wrong or expired; `errors` names the field `code`.", body = Problem),
    )
)]
async fn regenerate_recovery_codes(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    ValidJson(input): ValidJson<CodeRequest>,
) -> Result<Data<RecoveryCodesResponse>, Error> {
    require_session(&principal)?;
    let recovery_codes =
        state.mfa.regenerate_recovery_codes(principal.user_id(), &input.code).await?;
    Ok(Data { data: RecoveryCodesResponse { recovery_codes } })
}

/// Turn off two-step sign-in.
///
/// Needs a current authenticator code or an unused recovery code, and a password
/// session. Remaining recovery codes are deleted.
#[utoipa::path(
    post,
    path = "/me/mfa/totp/disable",
    operation_id = "disable_totp",
    security(("bearer" = [])),
    request_body = CodeRequest,
    responses(
        (status = 200, description = "Two-step sign-in is off.", body = Data<MfaStatusResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The request used an API key; code `auth.session_required`.", body = Problem),
        (status = 409, description = "Two-step sign-in is not on; code `auth.mfa_not_enabled`.", body = Problem),
        (status = 422, description = "The code is wrong or expired; `errors` names the field `code`.", body = Problem),
    )
)]
async fn disable_totp(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    ValidJson(input): ValidJson<CodeRequest>,
) -> Result<Data<MfaStatusResponse>, Error> {
    require_session(&principal)?;
    state.mfa.disable_totp(principal.user_id(), &input.code).await?;
    Ok(Data { data: state.mfa.status(principal.user_id()).await?.into() })
}

/// Turn off two-step sign-in for a user who lost their device.
///
/// Requires `auth.users.manage`. The user signs in with the password alone and can
/// set up a new device. Check who is asking before using this.
#[utoipa::path(
    delete,
    path = "/users/{id}/mfa",
    operation_id = "reset_user_mfa",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the user.")),
    responses(
        (status = 204, description = "Two-step sign-in is off for this user (also when it was not on)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `auth.users.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No user has this id; code `auth.user_not_found`.", body = Problem),
    )
)]
async fn reset_user_mfa(
    State(state): State<Arc<HttpState>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&USERS_MANAGE)?;
    state.users.get(id).await?.ok_or_else(|| crate::user::user_not_found(id))?;
    state.mfa.reset(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use autoxide_testkit::TestDatabase;
    use axum::http::StatusCode;
    use serde_json::json;

    use crate::http::test_support::{PASSWORD, app, bearer, call};
    use crate::mfa::service::tests::code_at;

    #[tokio::test]
    async fn bat_va_tat_mfa_qua_api() {
        TestDatabase::run(async |test| {
            let (app, _) = app(test).await;
            let ada = bearer(&app, "ada@example.com", PASSWORD).await;

            let (status, _, setup) = call(&app, "POST", "/me/mfa/totp", Some(&ada), None).await;
            assert_eq!(status, StatusCode::OK, "{setup}");
            let secret = setup["data"]["secret"].as_str().unwrap().to_owned();
            let now = u64::try_from(chrono::Utc::now().timestamp()).unwrap();

            let body = json!({"code": "000000"});
            let (status, _, wrong) =
                call(&app, "POST", "/me/mfa/totp/confirm", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}");

            let body = json!({"code": code_at(&secret, now)});
            let (status, _, on) =
                call(&app, "POST", "/me/mfa/totp/confirm", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{on}");
            assert_eq!(on["data"]["recovery_codes"].as_array().unwrap().len(), 10);
            let (_, _, status_body) = call(&app, "GET", "/me/mfa", Some(&ada), None).await;
            assert_eq!(status_body["data"]["totp_enabled"], true);
            assert_eq!(status_body["data"]["recovery_codes_remaining"], 10);

            let body = json!({"code": code_at(&secret, now + 30)});
            let (status, _, renewed) =
                call(&app, "POST", "/me/mfa/recovery-codes", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{renewed}");
            assert_ne!(renewed["data"]["recovery_codes"], on["data"]["recovery_codes"]);

            // Tắt bằng một recovery code (như khi mất điện thoại).
            let body = json!({"code": renewed["data"]["recovery_codes"][0]});
            // Bật rồi thì đăng nhập phải qua hai bước.
            let credentials = json!({"email": "ada@example.com", "password": PASSWORD});
            let (status, _, login) = call(&app, "POST", "/login", None, Some(credentials)).await;
            assert_eq!(status, StatusCode::ACCEPTED, "{login}");
            assert!(login["data"].get("access_token").is_none());
            assert_eq!(login["data"]["methods"], json!(["totp", "recovery_code"]));
            let second = json!({
                "mfa_token": login["data"]["mfa_token"],
                "code": renewed["data"]["recovery_codes"][1],
            });
            let (status, _, tokens) = call(&app, "POST", "/login/mfa", None, Some(second)).await;
            assert_eq!(status, StatusCode::OK, "{tokens}");
            assert!(tokens["data"]["access_token"].is_string());

            let (status, _, off) =
                call(&app, "POST", "/me/mfa/totp/disable", Some(&ada), Some(body)).await;
            assert_eq!(status, StatusCode::OK, "{off}");
            assert_eq!(off["data"]["totp_enabled"], false);
        })
        .await
        .unwrap();
    }
}
