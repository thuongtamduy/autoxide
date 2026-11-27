//! Route REST của định danh (KYC), dưới `/api/v1/compliance`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::identity::{PartyType, VerificationMethod};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext, root_of};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{KycSession, Subject};
use crate::entity::kyc_profile;
use crate::kyc::Decision;
use crate::service::ComplianceService;

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(my_kyc, open_my_kyc))
        .routes(routes!(start_my_session))
        .routes(routes!(submit_my_documents))
        .routes(routes!(list_profiles, open_profile))
        .routes(routes!(get_profile))
        .routes(routes!(start_session))
        .routes(routes!(submit_documents))
        .routes(routes!(approve))
        .routes(routes!(reject))
        .routes(routes!(revoke))
        .routes(routes!(request_info))
        .routes(routes!(receive_callback))
        .with_state(service)
}

/// An identity verification (KYC) profile.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceKycProfile)]
pub(crate) struct ProfileResponse {
    id: Uuid,
    /// `user`, `org`, `email` or `reference`.
    subject_kind: String,
    subject_id: String,
    /// `individual`, `household_business` or `organization`.
    party_type: String,
    /// `draft`, `pending`, `needs_info`, `verified`, `rejected`, `expired` or `revoked`.
    status: String,
    method: Option<String>,
    provider: Option<String>,
    verified_name: Option<String>,
    document_kind: Option<String>,
    /// Last four characters of the document number (the number itself is never kept).
    document_last4: Option<String>,
    assurance: Option<String>,
    /// Files sent for a manual review.
    document_file_ids: Vec<Uuid>,
    /// What the reviewer asked for (`needs_info`): `fields` and `message`.
    #[schema(value_type = Option<Object>)]
    info_request: Option<serde_json::Value>,
    verified_at: Option<Timestamp>,
    expires_at: Option<Timestamp>,
    reason_code: Option<String>,
    /// What the reviewer wrote to the person when refusing or revoking.
    review_note: Option<String>,
    /// `true` when the profile waits for the person (`draft` or `needs_info`) and no document has
    /// been sent for a manual review.
    missing_documents: bool,
    /// Name of the subject as known before verification: the account name for `user`, the unit
    /// name for `org`; `null` for other kinds, or when unknown. Compare with `verified_name`.
    subject_name: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl ProfileResponse {
    fn named(mut self, names: &std::collections::HashMap<(String, String), String>) -> Self {
        self.subject_name =
            names.get(&(self.subject_kind.clone(), self.subject_id.clone())).cloned();
        self
    }
}

impl From<kyc_profile::Model> for ProfileResponse {
    fn from(row: kyc_profile::Model) -> Self {
        let document_file_ids: Vec<Uuid> =
            serde_json::from_str(&row.document_files).unwrap_or_default();
        // Tính trước khi `row.status` bị chuyển vào struct (ownership: sau khi chuyển thì không mượn
        // lại được).
        let missing_documents =
            matches!(row.status.as_str(), "draft" | "needs_info") && document_file_ids.is_empty();
        ProfileResponse {
            id: row.id,
            subject_kind: row.subject_kind,
            subject_id: row.subject_id,
            party_type: row.party_type,
            status: row.status,
            method: row.method,
            provider: row.provider,
            verified_name: row.verified_name,
            document_kind: row.document_kind,
            document_last4: row.document_last4,
            assurance: row.assurance,
            document_file_ids,
            info_request: row.info_request.and_then(|text| serde_json::from_str(&text).ok()),
            verified_at: row.verified_at,
            expires_at: row.expires_at,
            missing_documents,
            reason_code: row.reason_code,
            review_note: row.review_note,
            subject_name: None,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

fn party(raw: Option<String>) -> Result<PartyType, Error> {
    match raw {
        None => Ok(PartyType::Individual),
        Some(raw) => PartyType::parse(&raw).ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                "party_type",
                "invalid",
                "must be `individual`, `household_business` or `organization`",
            )])
        }),
    }
}

fn method(raw: Option<String>) -> Result<Option<VerificationMethod>, Error> {
    match raw {
        None => Ok(None),
        Some(raw) => VerificationMethod::parse(&raw).map(Some).ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                "method",
                "invalid",
                "must be `vneid`, `ekyc` or `business_registry`",
            )])
        }),
    }
}

/// Your verification profile, or none yet.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceMyKyc)]
pub(crate) struct MyKyc {
    /// `none` when you have no profile yet.
    status: String,
    profile: Option<ProfileResponse>,
}

/// A profile to open.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceKycProfileRequest)]
pub(crate) struct ProfileRequest {
    /// The unit (seller) to verify; default the tenant's root. Ignored for `POST /me/kyc`.
    org_id: Option<Uuid>,
    /// `individual` (default), `household_business` or `organization`.
    party_type: Option<String>,
}

/// A verification to start.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceKycSessionRequest)]
pub(crate) struct SessionRequest {
    /// `vneid`, `ekyc` or `business_registry`; default the provider's first method.
    method: Option<String>,
    /// Page to come back to after the provider's steps.
    return_url: Option<String>,
}

/// Get your identity verification.
///
/// Any signed-in user.
#[utoipa::path(
    get,
    path = "/me/kyc",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_kyc",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Your verification.", body = Data<MyKyc>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn my_kyc(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<MyKyc>, Error> {
    let (status, profile) = service.my_kyc(&tenant, &principal).await?;
    Ok(Data { data: MyKyc { status, profile: profile.map(ProfileResponse::from) } })
}

/// Open your identity verification profile.
///
/// Any signed-in user (people who livestream or do affiliate marketing must be verified, Law on
/// E-commerce 2025 Article 17). Opening again returns the same profile.
#[utoipa::path(
    post,
    path = "/me/kyc",
    extensions(("x-permissions" = json!([]))),
    operation_id = "open_my_kyc",
    security(("bearer" = [])),
    request_body = ProfileRequest,
    responses(
        (status = 200, description = "Your profile.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn open_my_kyc(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ProfileRequest>,
) -> Result<Data<ProfileResponse>, Error> {
    let row = service
        .open_profile(&tenant, &Subject::User(principal.user_id()), party(input.party_type)?)
        .await?;
    Ok(Data { data: row.into() })
}

/// Start verifying your identity with the provider.
///
/// Any signed-in user with a profile (`POST /me/kyc`). Returns where to go next: the provider's page
/// (`redirect`), a token for the mobile SDK (`sdk`), or nothing to do (`pending`).
#[utoipa::path(
    post,
    path = "/me/kyc/sessions",
    extensions(("x-permissions" = json!([]))),
    operation_id = "start_my_kyc_session",
    security(("bearer" = [])),
    request_body = SessionRequest,
    responses(
        (status = 200, description = "Where to continue.", body = Data<KycSession>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No profile yet; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Already verified or revoked; code `compliance.kyc_state_invalid`.", body = Problem),
        (status = 422, description = "The method is not offered; every problem in `errors`.", body = Problem),
        (status = 503, description = "No identity provider is set up; code `identity_verification_disabled`.", body = Problem),
    )
)]
async fn start_my_session(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SessionRequest>,
) -> Result<Data<KycSession>, Error> {
    let (_, profile) = service.my_kyc(&tenant, &principal).await?;
    let profile = profile.ok_or_else(crate::kyc::kyc_not_found)?;
    let session = service
        .start_session(
            &tenant,
            Actor::from(&principal),
            profile.id,
            method(input.method)?,
            input.return_url,
        )
        .await?;
    Ok(Data { data: session })
}

/// Send your documents for a manual review.
///
/// Any signed-in user with a profile (`POST /me/kyc`). For servers without an identity provider: a
/// reviewer checks the files. No face images or biometrics are kept.
#[utoipa::path(
    post,
    path = "/me/kyc/documents",
    extensions(("x-permissions" = json!([]))),
    operation_id = "submit_my_kyc_documents",
    security(("bearer" = [])),
    request_body = DocumentsRequest,
    responses(
        (status = 200, description = "Your profile, now `pending`.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No profile yet; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Already verified or revoked; code `compliance.kyc_state_invalid`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn submit_my_documents(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<DocumentsRequest>,
) -> Result<Data<ProfileResponse>, Error> {
    let (_, profile) = service.my_kyc(&tenant, &principal).await?;
    let profile = profile.ok_or_else(crate::kyc::kyc_not_found)?;
    let row = service
        .submit_documents(&tenant, Actor::from(&principal), profile.id, input.file_ids)
        .await?;
    Ok(Data { data: row.into() })
}

/// Which profiles.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ProfilesQuery {
    /// Only profiles with this status.
    status: Option<String>,
}

/// List identity verification profiles.
///
/// Requires `compliance.kyc.read`.
#[utoipa::path(
    get,
    path = "/kyc/profiles",
    extensions(("x-permissions" = json!(["compliance.kyc.read"]))),
    operation_id = "list_kyc_profiles",
    security(("bearer" = [])),
    params(ProfilesQuery, PageQuery),
    responses(
        (status = 200, description = "One page of profiles.", body = Page<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_profiles(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ProfilesQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ProfileResponse>, Error> {
    // Quyền trước tham số trang: thiếu quyền là 403, không phải 422 (luật `permission-first` của xtask).
    principal.require(&crate::permissions::KYC_READ)?;
    let page = service
        .list_profiles(&tenant, &principal, query.status, &PageRequest::try_from(page)?)
        .await?;
    let names = service.subject_names(&tenant, &page.data).await;
    Ok(page.map(|row| ProfileResponse::from(row).named(&names)))
}

/// Open the identity verification profile of a unit (a seller).
///
/// Requires `compliance.kyc.submit` on the unit. Opening again returns the same profile.
#[utoipa::path(
    post,
    path = "/kyc/profiles",
    extensions(("x-permissions" = json!(["compliance.kyc.submit"]))),
    operation_id = "open_kyc_profile",
    security(("bearer" = [])),
    request_body = ProfileRequest,
    responses(
        (status = 201, description = "The profile.", body = Data<ProfileResponse>, headers(("Location" = String, description = "Path of the profile."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.submit` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn open_profile(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ProfileRequest>,
) -> Result<Created<ProfileResponse>, Error> {
    let org = input.org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    let row = service.open_org_profile(&tenant, &principal, org, party(input.party_type)?).await?;
    Ok(Created::new(format!("/compliance/kyc/profiles/{}", row.id), row.into()))
}

/// Get an identity verification profile.
///
/// Requires `compliance.kyc.read`.
#[utoipa::path(
    get,
    path = "/kyc/profiles/{id}",
    extensions(("x-permissions" = json!(["compliance.kyc.read"]))),
    operation_id = "get_kyc_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    responses(
        (status = 200, description = "The profile.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such profile; code `compliance.kyc_not_found`.", body = Problem),
    )
)]
async fn get_profile(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ProfileResponse>, Error> {
    let row = service.profile_for(&tenant, &principal, id).await?;
    let names = service.subject_names(&tenant, std::slice::from_ref(&row)).await;
    Ok(Data { data: ProfileResponse::from(row).named(&names) })
}

/// Start a verification of a unit's profile with the provider.
///
/// Requires `compliance.kyc.submit` on the unit.
#[utoipa::path(
    post,
    path = "/kyc/profiles/{id}/sessions",
    extensions(("x-permissions" = json!(["compliance.kyc.submit"]))),
    operation_id = "start_kyc_session",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    request_body = SessionRequest,
    responses(
        (status = 200, description = "Where to continue.", body = Data<KycSession>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.submit`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such profile you may submit; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Already verified or revoked; code `compliance.kyc_state_invalid`.", body = Problem),
        (status = 503, description = "No identity provider is set up; code `identity_verification_disabled`.", body = Problem),
    )
)]
async fn start_session(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SessionRequest>,
) -> Result<Data<KycSession>, Error> {
    principal.require(&crate::permissions::KYC_SUBMIT)?;
    let profile = service.submittable(&tenant, &principal, id).await?;
    let session = service
        .start_session(
            &tenant,
            Actor::from(&principal),
            profile.id,
            method(input.method)?,
            input.return_url,
        )
        .await?;
    Ok(Data { data: session })
}

/// Files for a manual review.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceKycDocumentsRequest)]
pub(crate) struct DocumentsRequest {
    /// Ids of files uploaded with `POST /api/v1/files` (ID card, business registration), 1 to 10.
    file_ids: Vec<Uuid>,
}

/// Send documents for a manual review.
///
/// Requires `compliance.kyc.submit` on the unit (people send their own with
/// `POST /me/kyc/documents`). A reviewer checks the files. No face images or biometrics are kept.
#[utoipa::path(
    post,
    path = "/kyc/profiles/{id}/documents",
    extensions(("x-permissions" = json!(["compliance.kyc.submit"]))),
    operation_id = "submit_kyc_documents",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    request_body = DocumentsRequest,
    responses(
        (status = 200, description = "The profile, now `pending`.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.submit`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such profile you may submit; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Already verified or revoked; code `compliance.kyc_state_invalid`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn submit_documents(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<DocumentsRequest>,
) -> Result<Data<ProfileResponse>, Error> {
    principal.require(&crate::permissions::KYC_SUBMIT)?;
    let profile = service.submittable(&tenant, &principal, id).await?;
    let row = service
        .submit_documents(&tenant, Actor::from(&principal), profile.id, input.file_ids)
        .await?;
    Ok(Data { data: row.into() })
}

/// Why a profile is refused or revoked.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceKycReasonRequest)]
pub(crate) struct ReasonRequest {
    /// Short code, e.g. `document_unreadable`, `name_mismatch`, `fraud_suspected`.
    reason_code: String,
    /// Note for the person, up to 1000 characters: what was wrong and what to do.
    note: Option<String>,
}

/// What more is needed.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceKycInfoRequest)]
pub(crate) struct InfoRequest {
    /// Fields or documents to send again.
    fields: Vec<String>,
    /// Message for the person, up to 1000 characters.
    message: String,
}

/// Mã lý do và ghi chú, kiểm cả hai rồi báo một lần.
fn reason(input: ReasonRequest) -> Result<(String, Option<String>), Error> {
    let mut problems = Vec::new();
    let code = input.reason_code.trim().to_owned();
    if !(1..=64).contains(&code.len()) {
        problems.push(FieldError::new("reason_code", "length", "must be 1 to 64 characters"));
    }
    let note = input.note.map(|note| note.trim().to_owned()).filter(|note| !note.is_empty());
    if note.as_ref().is_some_and(|note| note.chars().count() > 1000) {
        problems.push(FieldError::new("note", "length", "must be at most 1000 characters"));
    }
    if problems.is_empty() { Ok((code, note)) } else { Err(Error::validation(problems)) }
}

/// Approve a profile.
///
/// Requires `compliance.kyc.review`, signed in as a person (API keys cannot).
#[utoipa::path(
    post,
    path = "/kyc/profiles/{id}/approve",
    extensions(("x-permissions" = json!(["compliance.kyc.review"]))),
    operation_id = "approve_kyc_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    responses(
        (status = 200, description = "The profile, now `verified`.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.review` (`permission_denied`) or not a person (`compliance.session_required`).", body = Problem),
        (status = 404, description = "No such profile; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Not waiting for a review; code `compliance.kyc_state_invalid`.", body = Problem),
    )
)]
async fn approve(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ProfileResponse>, Error> {
    Ok(Data { data: service.review(&tenant, &principal, id, Decision::Approve).await?.into() })
}

/// Refuse a profile.
///
/// Requires `compliance.kyc.review`, signed in as a person.
#[utoipa::path(
    post,
    path = "/kyc/profiles/{id}/reject",
    extensions(("x-permissions" = json!(["compliance.kyc.review"]))),
    operation_id = "reject_kyc_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    request_body = ReasonRequest,
    responses(
        (status = 200, description = "The profile, now `rejected`.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.review` (`permission_denied`) or not a person (`compliance.session_required`).", body = Problem),
        (status = 404, description = "No such profile; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Not waiting for a review; code `compliance.kyc_state_invalid`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn reject(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReasonRequest>,
) -> Result<Data<ProfileResponse>, Error> {
    principal.require(&crate::permissions::KYC_REVIEW)?;
    let (code, note) = reason(input)?;
    let decision = Decision::Reject(code, note);
    Ok(Data { data: service.review(&tenant, &principal, id, decision).await?.into() })
}

/// Revoke a verified profile.
///
/// Requires `compliance.kyc.review`, signed in as a person.
#[utoipa::path(
    post,
    path = "/kyc/profiles/{id}/revoke",
    extensions(("x-permissions" = json!(["compliance.kyc.review"]))),
    operation_id = "revoke_kyc_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    request_body = ReasonRequest,
    responses(
        (status = 200, description = "The profile, now `revoked`.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.review` (`permission_denied`) or not a person (`compliance.session_required`).", body = Problem),
        (status = 404, description = "No such profile; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Not verified; code `compliance.kyc_state_invalid`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn revoke(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReasonRequest>,
) -> Result<Data<ProfileResponse>, Error> {
    principal.require(&crate::permissions::KYC_REVIEW)?;
    let (code, note) = reason(input)?;
    let decision = Decision::Revoke(code, note);
    Ok(Data { data: service.review(&tenant, &principal, id, decision).await?.into() })
}

/// Ask for more information.
///
/// Requires `compliance.kyc.review`, signed in as a person. The profile goes to `needs_info`; sending
/// documents or starting a verification again puts it back to `pending`.
#[utoipa::path(
    post,
    path = "/kyc/profiles/{id}/request-info",
    extensions(("x-permissions" = json!(["compliance.kyc.review"]))),
    operation_id = "request_kyc_info",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    request_body = InfoRequest,
    responses(
        (status = 200, description = "The profile, now `needs_info`.", body = Data<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.kyc.review` (`permission_denied`) or not a person (`compliance.session_required`).", body = Problem),
        (status = 404, description = "No such profile; code `compliance.kyc_not_found`.", body = Problem),
        (status = 409, description = "Not waiting for a review; code `compliance.kyc_state_invalid`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn request_info(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<InfoRequest>,
) -> Result<Data<ProfileResponse>, Error> {
    principal.require(&crate::permissions::KYC_REVIEW)?;
    if input.fields.is_empty()
        || input.message.trim().is_empty()
        || input.message.chars().count() > 1000
    {
        return Err(Error::validation(vec![FieldError::new(
            "message",
            "required",
            "name at least one field and write a message of up to 1000 characters",
        )]));
    }
    let decision = Decision::RequestInfo { fields: input.fields, message: input.message };
    Ok(Data { data: service.review(&tenant, &principal, id, decision).await?.into() })
}

/// Receive a result from the identity provider.
///
/// No sign-in: the provider signs every callback; the signature is checked on the raw body before
/// anything is stored. The result is processed in the background.
#[utoipa::path(
    post,
    path = "/kyc/callbacks/{provider}",
    extensions(("x-public" = json!(true))),
    operation_id = "receive_kyc_callback",
    params(("provider" = String, Path, description = "Provider name.")),
    request_body(content = String, description = "The provider's callback, as sent.", content_type = "application/json"),
    responses(
        (status = 204, description = "Stored."),
        (status = 401, description = "The signature is not valid; code `identity_callback_invalid`.", body = Problem),
        (status = 503, description = "No such provider is set up; code `identity_verification_disabled`.", body = Problem),
    )
)]
async fn receive_callback(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    Path(provider): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, Error> {
    service.receive_callback(&tenant, &provider, &headers, &body).await?;
    Ok(StatusCode::NO_CONTENT)
}
