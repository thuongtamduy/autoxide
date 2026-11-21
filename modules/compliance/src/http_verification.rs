//! Route REST của xác thực tài khoản đăng nội dung (NĐ 147/2024, D-356), dưới `/api/v1/compliance`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::VerificationLevel;
use crate::entity::phone_challenge;
use crate::service::ComplianceService;
use crate::verification::Verification;

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(my_verification))
        .routes(routes!(start_phone_verification))
        .routes(routes!(get_phone_verification))
        .routes(routes!(confirm_phone_verification))
        .routes(routes!(remove_phone))
        .with_state(service)
}

/// How far your account is verified, for posting content (Vietnam, Decree 147/2024/ND-CP).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceAccountVerification)]
pub(crate) struct VerificationResponse {
    /// `none`, `phone` (a mobile number confirmed with a code) or `identity` (identity verified; it
    /// includes `phone`).
    pub level: VerificationLevel,
    /// The verified phone number, masked (e.g. `+8491****678`).
    pub phone: Option<String>,
    pub phone_verified_at: Option<Timestamp>,
    pub identity_verified_at: Option<Timestamp>,
}

impl From<Verification> for VerificationResponse {
    fn from(found: Verification) -> Self {
        VerificationResponse {
            level: found.level,
            phone: found.phone.as_ref().map(autoxide_kernel::sms::PhoneNumber::masked),
            phone_verified_at: found.phone_verified_at,
            identity_verified_at: found.identity_verified_at,
        }
    }
}

/// A phone number to verify.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CompliancePhoneVerificationRequest)]
pub(crate) struct PhoneRequest {
    /// A mobile number, e.g. `0912 345 678` or `+84912345678`.
    phone: String,
}

/// A code to confirm.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CompliancePhoneConfirmRequest)]
pub(crate) struct ConfirmRequest {
    /// The 6-digit code from the SMS.
    code: String,
}

/// A one-time code sent by SMS.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CompliancePhoneVerification)]
pub(crate) struct ChallengeResponse {
    id: Uuid,
    /// The number the code went to, masked.
    phone: String,
    expires_at: Timestamp,
    /// Wrong codes entered so far.
    attempts: i32,
    /// `true` once the code was used.
    confirmed: bool,
    created_at: Timestamp,
}

impl From<phone_challenge::Model> for ChallengeResponse {
    fn from(row: phone_challenge::Model) -> Self {
        let phone = autoxide_kernel::sms::PhoneNumber::parse(&row.phone, "84")
            .map_or_else(|| "****".to_owned(), |phone| phone.masked());
        ChallengeResponse {
            id: row.id,
            phone,
            expires_at: row.expires_at,
            attempts: row.attempts,
            confirmed: row.consumed_at.is_some(),
            created_at: row.created_at,
        }
    }
}

/// Get your account verification level.
///
/// Any signed-in user. `identity` comes from a verified KYC profile (`/me/kyc`); `phone` from
/// `POST /me/phone-verifications`.
#[utoipa::path(
    get,
    path = "/me/verification",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_account_verification",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Your verification.", body = Data<VerificationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn my_verification(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<VerificationResponse>, Error> {
    let found = service.verification(&tenant, principal.user_id()).await?;
    Ok(Data { data: found.into() })
}

/// Send a code to verify a phone number.
///
/// Any signed-in user. Sends a 6-digit one-time code by SMS to a Vietnamese mobile number; confirm
/// it with `POST /me/phone-verifications/{id}/confirm` before `expires_at`. Limited per number,
/// per account and per address.
#[utoipa::path(
    post,
    path = "/me/phone-verifications",
    extensions(("x-permissions" = json!([]))),
    operation_id = "start_phone_verification",
    security(("bearer" = [])),
    request_body = PhoneRequest,
    responses(
        (status = 201, description = "The code was sent.", body = Data<ChallengeResponse>, headers(("Location" = String, description = "Path of the verification."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 409, description = "The number is verified for another account; code `compliance.phone_in_use`.", body = Problem),
        (status = 422, description = "Not a phone number (`errors`), or not an accepted mobile number; code `compliance.phone_not_allowed`.", body = Problem),
        (status = 429, description = "Too many codes asked for; code `compliance.otp_throttled`, wait `Retry-After` seconds.", body = Problem),
        (status = 503, description = "Sending SMS is not set up (code `sms_disabled`) or the provider failed; try again later.", body = Problem),
    )
)]
async fn start_phone_verification(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    client: ClientInfo,
    ValidJson(input): ValidJson<PhoneRequest>,
) -> Result<Created<ChallengeResponse>, Error> {
    let row =
        service.start_phone_verification(&tenant, &principal, &input.phone, client.ip).await?;
    Ok(Created::new(format!("/compliance/me/phone-verifications/{}", row.id), row.into()))
}

/// Get one of your phone verifications.
///
/// Any signed-in user; only their own.
#[utoipa::path(
    get,
    path = "/me/phone-verifications/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_phone_verification",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Verification ID.")),
    responses(
        (status = 200, description = "The verification.", body = Data<ChallengeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such verification of yours; code `compliance.challenge_not_found`.", body = Problem),
    )
)]
async fn get_phone_verification(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ChallengeResponse>, Error> {
    let row = service.phone_challenge(&tenant, &principal, id).await?;
    Ok(Data { data: row.into() })
}

/// Confirm a phone number with the code.
///
/// Any signed-in user; only their own verification. A code works once, before it expires, and
/// only a few wrong tries are allowed. The number then becomes your verified phone (replacing an
/// earlier one).
#[utoipa::path(
    post,
    path = "/me/phone-verifications/{id}/confirm",
    extensions(("x-permissions" = json!([]))),
    operation_id = "confirm_phone_verification",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Verification ID.")),
    request_body = ConfirmRequest,
    responses(
        (status = 200, description = "Your verification after the confirmation.", body = Data<VerificationResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such verification of yours; code `compliance.challenge_not_found`.", body = Problem),
        (status = 409, description = "The number was verified for another account meanwhile; code `compliance.phone_in_use`.", body = Problem),
        (status = 422, description = "Wrong, used or expired code; code `compliance.otp_invalid`.", body = Problem),
        (status = 429, description = "Too many wrong codes; code `compliance.otp_attempts_exceeded` — ask for a new code.", body = Problem),
    )
)]
async fn confirm_phone_verification(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ConfirmRequest>,
) -> Result<Data<VerificationResponse>, Error> {
    let found = service.confirm_phone(&tenant, &principal, id, &input.code).await?;
    Ok(Data { data: found.into() })
}

/// Remove your verified phone number.
///
/// Any signed-in user. Calling it again does nothing. Places that need phone verification refuse
/// your posts until you verify a number again.
#[utoipa::path(
    delete,
    path = "/me/phone",
    extensions(("x-permissions" = json!([]))),
    operation_id = "remove_my_phone",
    security(("bearer" = [])),
    responses(
        (status = 204, description = "Removed (or there was none)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn remove_phone(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<StatusCode, Error> {
    service.remove_phone(&tenant, &principal).await?;
    Ok(StatusCode::NO_CONTENT)
}
