//! REST của khách tự đăng ký (D-349): ba endpoint công khai cho người đăng ký, một danh sách cho nhân
//! sự nền tảng. Handler chỉ lấy dữ liệu vào → gọi service (`signup.rs`) → trả envelope.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::http::operator;
use crate::permissions::SUBSCRIPTIONS_READ;
use crate::service::PlansService;
use crate::signup::{STATUSES, SignupRequest, StartedKyc, entity};

pub(crate) fn routes() -> OpenApiRouter<Arc<PlansService>> {
    OpenApiRouter::new()
        .routes(routes!(list_signups, create_signup))
        .routes(routes!(confirm_signup))
        .routes(routes!(get_signup))
}

/// A tenant sign-up, as its applicant (with the token from the mail) or platform staff see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SignupResponse {
    id: Uuid,
    /// `pending_email` (waiting for the link in the mail), `awaiting_kyc` (waiting for identity
    /// checks), `provisioning` (the tenant is being created), `completed` (see `tenant_id`; the
    /// administrator got an invitation by mail), `rejected` (see `reason`) or `expired`.
    #[schema(example = "provisioning")]
    status: String,
    email: String,
    admin_name: Option<String>,
    tenant_name: String,
    /// Short name of the new tenant.
    slug: String,
    plan_id: Uuid,
    price_id: Option<Uuid>,
    /// `individual`, `household_business` or `organization`.
    party_type: String,
    /// Language of the mails: `en` or `vi`.
    locale: Option<String>,
    /// Why it was rejected: `kyc_rejected`, `tenant_slug_taken`, or the code of the error that
    /// stopped the tenant from being created.
    reason: Option<String>,
    /// The new tenant, once `completed`.
    tenant_id: Option<Uuid>,
    /// The provisioning creating the tenant (`GET /provisionings/{id}` for platform staff).
    provisioning_id: Option<Uuid>,
    /// Identity checks to do now: only in the answer of `POST /signups/confirm` while
    /// `awaiting_kyc`.
    kyc: Option<SignupKyc>,
    confirmed_at: Option<DateTime<Utc>>,
    /// Until when it waits for the mail link (`pending_email`) or the identity checks
    /// (`awaiting_kyc`).
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// Where the applicant goes for the identity checks.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SignupKyc {
    /// `redirect` (open `url`), `sdk` (start the provider's mobile SDK with `token`) or `pending`
    /// (nothing to do; the result comes later).
    kind: String,
    url: Option<String>,
    token: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

impl SignupResponse {
    pub(crate) fn of(row: entity::Model, kyc: Option<StartedKyc>) -> Self {
        SignupResponse {
            id: row.id,
            status: row.status,
            email: row.email,
            admin_name: row.admin_name,
            tenant_name: row.tenant_name,
            slug: row.slug,
            plan_id: row.plan_id,
            price_id: row.price_id,
            party_type: row.party_type,
            locale: row.locale,
            reason: row.reason,
            tenant_id: row.tenant_id,
            provisioning_id: row.provisioning_id,
            kyc: kyc.map(|kyc| SignupKyc {
                kind: kyc.kind,
                url: kyc.url,
                token: kyc.token,
                expires_at: kyc.expires_at,
            }),
            confirmed_at: row.confirmed_at,
            expires_at: row.expires_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Sign up a new tenant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignupBody {
    /// Email of the person signing up: they confirm it, then administer the new tenant.
    #[schema(example = "owner@acme.example")]
    email: String,
    /// Their name, up to 200 characters.
    admin_name: Option<String>,
    /// Name of the new tenant, 1 to 200 characters.
    #[schema(example = "Acme Ltd")]
    tenant_name: String,
    /// Short name of the new tenant: 1 to 63 lower-case letters, digits and `-`.
    #[schema(example = "acme")]
    slug: String,
    /// Code of an active base plan open to sign-up (`self_signup` in `GET /catalog`).
    #[schema(example = "starter")]
    plan_code: String,
    /// An active recurring price of that plan; default: none chosen yet.
    price_id: Option<Uuid>,
    /// Who signs up, for plans that need identity checks: `individual`, `household_business` or
    /// `organization` (default).
    party_type: Option<String>,
    /// Language of the mails: `en` (default) or `vi`.
    locale: Option<String>,
    /// Must be `true`: the person accepts the current terms of use and privacy policy of the
    /// platform (`GET /api/v1/compliance/documents`). The acceptance is recorded when the email
    /// is confirmed.
    accept_terms: bool,
    /// Token from `GET /api/v1/antispam/token?scope=plans:signup`.
    form_token: Option<String>,
    /// The hidden trap field: people leave it empty.
    trap: Option<String>,
    /// The value of the `altcha` widget, when the server uses a challenge.
    challenge: Option<String>,
}

/// The answer to a sign-up: always the same.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SignupAccepted {
    /// Always `true`. Tell the person to open the link in the mail.
    accepted: bool,
}

/// The token from the confirmation mail.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignupConfirmBody {
    token: String,
}

/// The token from the confirmation mail.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SignupTokenQuery {
    /// The token from the confirmation mail.
    token: String,
}

/// Filters of the sign-up list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SignupsQuery {
    /// Only sign-ups in this state: `pending_email`, `awaiting_kyc`, `provisioning`, `completed`,
    /// `rejected` or `expired`.
    status: Option<String>,
}

/// Sign up a new tenant (public).
///
/// No sign-in needed; only when the server takes sign-ups (`AUTOXIDE_PLANS_SIGNUP`), on the platform
/// host, for plans marked `self_signup`. The answer is the same `202` whether the request is new,
/// repeats one still waiting (same email and slug: a new mail, the old link stops working), or was
/// dropped by the anti-spam checks. A background job then mails a link valid 24 hours; the person
/// opens it and the page sends the token to `POST /signups/confirm`. Send `form_token` (and
/// `challenge` when the server uses one). Two people waiting for the same `slug`: the first to get
/// to the provisioning keeps it, the other is `rejected` (`tenant_slug_taken`).
#[utoipa::path(
    post,
    path = "/signups",
    extensions(("x-public" = json!(true))),
    operation_id = "create_signup",
    request_body = SignupBody,
    responses(
        (status = 202, description = "Received; a confirmation mail follows.", body = Data<SignupAccepted>),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 404, description = "This server does not take sign-ups; code `plans.signup_disabled`.", body = Problem),
        (status = 422, description = "A value is invalid (e.g. `plan_code` not open to sign-up, `slug` taken, `accept_terms` not `true`); every problem in `errors`.", body = Problem),
        (status = 503, description = "The anti-spam check cannot run; code `antispam.verifier_unavailable`.", body = Problem),
    )
)]
async fn create_signup(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    client: ClientInfo,
    ValidJson(input): ValidJson<SignupBody>,
) -> Result<(StatusCode, Data<SignupAccepted>), Error> {
    let request = SignupRequest {
        email: input.email,
        admin_name: input.admin_name,
        tenant_name: input.tenant_name,
        slug: input.slug,
        plan_code: input.plan_code,
        price_id: input.price_id,
        party_type: input.party_type,
        locale: input.locale,
        accept_terms: input.accept_terms,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    service.submit_signup(&tenant, &client, request).await?;
    Ok((StatusCode::ACCEPTED, Data { data: SignupAccepted { accepted: true } }))
}

/// Confirm a tenant sign-up with the token from the mail (public).
///
/// No sign-in needed. Records that the person accepted the current terms of use and privacy policy.
/// When the plan needs identity checks the sign-up becomes `awaiting_kyc` and `kyc` says where to do
/// them (calling again while waiting starts new checks); otherwise the tenant is created in the
/// background (`provisioning`, then `completed`, and the person gets an invitation to administer
/// it). Calling again later answers the sign-up as it stands. Keep the token: it also reads the
/// sign-up (`GET /signups/{id}?token=`).
#[utoipa::path(
    post,
    path = "/signups/confirm",
    extensions(("x-public" = json!(true))),
    operation_id = "confirm_signup",
    request_body = SignupConfirmBody,
    responses(
        (status = 200, description = "The sign-up after confirming it.", body = Data<SignupResponse>),
        (status = 400, description = "The token is wrong, used up or expired (code `plans.signup_token_invalid`, `reason` `unknown` or `expired`): sign up again.", body = Problem),
        (status = 404, description = "This server does not take sign-ups; code `plans.signup_disabled`.", body = Problem),
        (status = 503, description = "Identity checks cannot start now (`plans.kyc_unavailable`, `compliance.verification_disabled`); try again later.", body = Problem),
    )
)]
async fn confirm_signup(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    ValidJson(input): ValidJson<SignupConfirmBody>,
) -> Result<Data<SignupResponse>, Error> {
    let (row, kyc) = service.confirm_signup(&tenant, &input.token).await?;
    Ok(Data { data: SignupResponse::of(row, kyc) })
}

/// Get a tenant sign-up with the token from the mail (public).
///
/// No sign-in needed: the token from the confirmation mail is the key. Use it to follow a sign-up
/// through the identity checks and the creation of the tenant.
#[utoipa::path(
    get,
    path = "/signups/{id}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_signup",
    params(("id" = Uuid, Path, description = "Id of the sign-up."), SignupTokenQuery),
    responses(
        (status = 200, description = "The sign-up.", body = Data<SignupResponse>),
        (status = 404, description = "No sign-up has this id and token (`plans.signup_not_found`), or the server does not take sign-ups (`plans.signup_disabled`).", body = Problem),
    )
)]
async fn get_signup(
    State(service): State<Arc<PlansService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    Query(query): Query<SignupTokenQuery>,
) -> Result<Data<SignupResponse>, Error> {
    let row = service.signup_status(&tenant, id, &query.token).await?;
    Ok(Data { data: SignupResponse::of(row, None) })
}

/// List tenant sign-ups.
///
/// Platform staff with `plans.subscriptions.read`. Newest first; rows that ended (`expired`,
/// `rejected`) are deleted 30 days later.
#[utoipa::path(
    get,
    path = "/signups",
    extensions(("x-permissions" = json!(["plans.subscriptions.read"]))),
    operation_id = "list_signups",
    security(("bearer" = [])),
    params(SignupsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of sign-ups, newest first.", body = Page<SignupResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `plans.subscriptions.read` (`permission_denied`).", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_signups(
    State(service): State<Arc<PlansService>>,
    principal: Principal,
    Query(filter): Query<SignupsQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<SignupResponse>, Error> {
    operator(&principal, &SUBSCRIPTIONS_READ)?;
    let page = PageRequest::try_from(page);
    let status = check_status(filter.status)?;
    let page = service.list_signups(status, &page?).await?;
    Ok(page.map(|row| SignupResponse::of(row, None)))
}

/// Kiểm bộ lọc `status` (dùng chung cho REST và MCP).
pub(crate) fn check_status(status: Option<String>) -> Result<Option<String>, Error> {
    if status.as_deref().is_some_and(|status| !STATUSES.contains(&status)) {
        return Err(Error::validation(vec![
            FieldError::new("status", "invalid", "must be one of {values}")
                .with_param("values", STATUSES.join(", ")),
        ]));
    }
    Ok(status)
}
