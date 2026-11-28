//! Route REST của dữ liệu cá nhân (D-360) dưới `/api/v1/compliance`: sổ mục đích, thời hạn lưu, sổ
//! hoạt động xử lý, nơi lưu, sự cố (mốc 72 giờ) và sổ yêu cầu của chủ thể qua kênh khác (D-217).

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::{incident, incident_entry, subject_request};
use crate::incidents::{IncidentChange, NewIncident};
use crate::permissions::PRIVACY_READ;
use crate::privacy::{
    ProcessingRegister, Purpose, PurposeInput, PurposeWithCounts, Recipient, ResidencyView,
    RetentionItem,
};
use crate::service::ComplianceService;
use crate::subject_requests::NewRequest;

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_purposes, create_purpose))
        .routes(routes!(update_purpose))
        .routes(routes!(public_purposes))
        .routes(routes!(retention))
        .routes(routes!(processing_register))
        .routes(routes!(get_residency, set_residency))
        .routes(routes!(list_incidents, open_incident))
        .routes(routes!(get_incident, update_incident))
        .routes(routes!(add_incident_entry))
        .routes(routes!(incident_draft))
        .routes(routes!(list_subject_requests, record_subject_request))
        .routes(routes!(get_subject_request, update_subject_request))
        .routes(routes!(fulfil_subject_request))
        .with_state(service)
}

// ---- Mục đích --------------------------------------------------------------------------------

/// A processing purpose to add.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CompliancePurposeRequest)]
pub(crate) struct PurposeRequest {
    /// Lower-case letters, digits and `_`, e.g. `loyalty`. A key of the default purposes overrides
    /// that default.
    pub key: String,
    /// `service`, `marketing`, `analytics`, `legal` or `kyc`.
    pub category: String,
    /// `consent`, `contract`, `legal_obligation`, `vital_interest` or `public_interest`.
    pub legal_basis: String,
    #[serde(default)]
    pub data_categories: Vec<String>,
    #[serde(default)]
    pub recipients: Vec<Recipient>,
    /// Description by language, e.g. `{"en": "…", "vi": "…"}`.
    #[schema(value_type = Object)]
    pub description: BTreeMap<String, String>,
    pub retention: Option<String>,
    pub active: Option<bool>,
}

/// Changes to a processing purpose (absent fields stay as they are).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CompliancePurposeChange)]
pub(crate) struct PurposeChange {
    pub category: Option<String>,
    pub legal_basis: Option<String>,
    pub data_categories: Option<Vec<String>>,
    pub recipients: Option<Vec<Recipient>>,
    #[schema(value_type = Option<Object>)]
    pub description: Option<BTreeMap<String, String>>,
    /// How long the data is kept, in words; `null` clears it.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<String>)]
    pub retention: Option<Option<String>>,
    pub active: Option<bool>,
}

fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// List the processing purposes of the tenant.
///
/// Requires `compliance.privacy.read`. The defaults of the legal rules (`privacy.default_purposes`)
/// merged with the purposes the tenant added or changed, each with the number of people who
/// consented and who withdrew. By key (A→Z unless `order=desc`); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/purposes",
    extensions(("x-permissions" = json!(["compliance.privacy.read"]))),
    operation_id = "list_compliance_purposes",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the purposes, with how many people consented and withdrew.", body = Page<PurposeWithCounts>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_purposes(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<PurposeWithCounts>, Error> {
    principal.require(&PRIVACY_READ)?;
    let page = PageRequest::try_from(page)?;
    service.list_purposes(&tenant, &principal, &page).await
}

/// Add a processing purpose.
///
/// Requires `compliance.privacy.manage`. Personal Data Protection Law 91/2025/QH15: each purpose
/// states its legal basis — have a legal professional review them.
#[utoipa::path(
    post,
    path = "/purposes",
    extensions(("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "create_compliance_purpose",
    security(("bearer" = [])),
    request_body = PurposeRequest,
    responses(
        (status = 201, description = "The purpose.", body = Data<Purpose>, headers(("Location" = String, description = "Path of the purposes."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The tenant already has this purpose (code `compliance.purpose_exists`), or already has `AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX` purposes of its own (code `compliance.purpose_limit_reached`).", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_purpose(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PurposeRequest>,
) -> Result<Created<Purpose>, Error> {
    let input = PurposeInput {
        key: Some(input.key),
        category: Some(input.category),
        legal_basis: Some(input.legal_basis),
        data_categories: Some(input.data_categories),
        recipients: Some(input.recipients),
        description: Some(input.description),
        retention: Some(input.retention),
        active: input.active,
    };
    let saved = service.create_purpose(&tenant, &principal, input).await?;
    Ok(Created::new("/compliance/purposes", saved))
}

/// Change a processing purpose.
///
/// Requires `compliance.privacy.manage`. Changing a default purpose stores the tenant's own version
/// of it.
#[utoipa::path(
    patch,
    path = "/purposes/{key}",
    extensions(("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "update_compliance_purpose",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the purpose.")),
    request_body = PurposeChange,
    responses(
        (status = 200, description = "The purpose after the change.", body = Data<Purpose>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such purpose; code `compliance.purpose_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn update_purpose(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    ValidJson(input): ValidJson<PurposeChange>,
) -> Result<Data<Purpose>, Error> {
    let input = PurposeInput {
        key: None,
        category: input.category,
        legal_basis: input.legal_basis,
        data_categories: input.data_categories,
        recipients: input.recipients,
        description: input.description,
        retention: input.retention,
        active: input.active,
    };
    Ok(Data { data: service.update_purpose(&tenant, &principal, &key, input).await? })
}

/// List the processing purposes, for the public privacy notice.
///
/// No sign-in (the tenant comes from the domain). Only purposes in use; the advertising and
/// analytics providers active on the tenant's websites are listed as recipients, with the country
/// that receives the data (transfers abroad). By key (A→Z unless `order=desc`); paged with
/// `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/public/purposes",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_compliance_purposes",
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the purposes in use.", body = Page<Purpose>),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn public_purposes(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<Purpose>, Error> {
    let page = PageRequest::try_from(page)?;
    service.public_purposes_page(&tenant, &page).await
}

// ---- Thời hạn lưu, sổ xử lý, nơi lưu ---------------------------------------------------------

/// List how long each kind of personal data is kept.
///
/// Requires `compliance.privacy.read`. Every module declares the personal data it keeps; each item
/// is compared with the limits of the legal rule `privacy.retention_limits`. By module, then name
/// (A→Z unless `order=desc`); paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/retention",
    extensions(("x-permissions" = json!(["compliance.privacy.read"]))),
    operation_id = "list_compliance_retention",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the kinds of personal data.", body = Page<RetentionItem>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn retention(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<RetentionItem>, Error> {
    principal.require(&PRIVACY_READ)?;
    let page = PageRequest::try_from(page)?;
    service.retention(&tenant, &principal, &page).await
}

/// Get the register of processing activities.
///
/// Requires `compliance.privacy.read`. Purposes, legal bases, kinds of data, recipients, retention
/// and where the data is kept — the input of a data protection impact assessment (its official form
/// needs a legal professional).
#[utoipa::path(
    get,
    path = "/processing-register",
    extensions(("x-permissions" = json!(["compliance.privacy.read"]))),
    operation_id = "get_compliance_processing_register",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The register.", body = Data<ProcessingRegister>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn processing_register(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<ProcessingRegister>, Error> {
    Ok(Data { data: service.processing_register(&tenant, &principal).await? })
}

/// Get where the tenant's data is kept.
///
/// Requires `compliance.privacy.read`.
#[utoipa::path(
    get,
    path = "/residency",
    extensions(("x-permissions" = json!(["compliance.privacy.read"]))),
    operation_id = "get_compliance_residency",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Where the data is kept.", body = Data<ResidencyView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn get_residency(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<ResidencyView>, Error> {
    Ok(Data { data: service.residency(&tenant, &principal).await? })
}

/// Where a tenant must keep its data.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceResidencyRequest)]
pub(crate) struct ResidencyRequest {
    /// The tenant; defaults to the caller's (the platform tenant).
    pub tenant_id: Option<Uuid>,
    /// Two-letter country code (e.g. `VN`: keep the data in Vietnam), or `null` for no requirement.
    pub data_residency: Option<String>,
}

/// Set where a tenant must keep its data.
///
/// Platform staff with `compliance.privacy.manage`. Refused when the tenant's placement is not in
/// that country (or its country is unknown): record it with `autoxide placement country` or move the
/// tenant first. Moving a tenant later to a placement elsewhere is refused too. Cybersecurity Law
/// 116/2025/QH15 and its implementing decree (replacing Decree 53/2022/ND-CP) — have a legal professional
/// check who must keep data in Vietnam.
#[utoipa::path(
    put,
    path = "/residency",
    extensions(("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "set_compliance_residency",
    security(("bearer" = [])),
    request_body = ResidencyRequest,
    responses(
        (status = 200, description = "Where the data is kept now.", body = Data<ResidencyView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage` (code `permission_denied`) or not platform staff (code `tenant_platform_only`).", body = Problem),
        (status = 404, description = "No such tenant; code `tenant_not_found`.", body = Problem),
        (status = 409, description = "The tenant's placement is not in that country; code `tenant_residency_violation`.", body = Problem),
        (status = 422, description = "Not a two-letter country code; every problem in `errors`.", body = Problem),
    )
)]
async fn set_residency(
    State(service): State<Arc<ComplianceService>>,
    principal: Principal,
    ValidJson(input): ValidJson<ResidencyRequest>,
) -> Result<Data<ResidencyView>, Error> {
    let found =
        service.set_residency(&principal, input.tenant_id, input.data_residency.as_deref()).await?;
    Ok(Data { data: found })
}

// ---- Sự cố -----------------------------------------------------------------------------------

/// A personal data breach.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceIncident)]
pub(crate) struct IncidentResponse {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    /// `low`, `medium`, `high` or `critical`.
    pub severity: String,
    /// `open`, `contained`, `reported` or `closed`.
    pub status: String,
    /// When it was detected; the reporting deadline counts from here.
    pub detected_at: Timestamp,
    /// Report to the authority before this time (72 hours by default, legal rule `privacy.incident`).
    pub report_due_at: Timestamp,
    /// `true` while not reported and past `report_due_at`.
    pub overdue: bool,
    pub authority_notified_at: Option<Timestamp>,
    pub authority_reference: Option<String>,
    pub subjects_notified_at: Option<Timestamp>,
    pub data_categories: Vec<String>,
    pub affected_count: Option<i64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// The log (`GET /incidents/{id}` only; absent in lists).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<IncidentEntryResponse>>,
}

/// A line of the log of a breach.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceIncidentEntry)]
pub(crate) struct IncidentEntryResponse {
    pub id: Uuid,
    pub body: String,
    pub author_user_id: Option<Uuid>,
    pub created_at: Timestamp,
}

impl From<incident_entry::Model> for IncidentEntryResponse {
    fn from(row: incident_entry::Model) -> Self {
        IncidentEntryResponse {
            id: row.id,
            body: row.body,
            author_user_id: row.author_user_id,
            created_at: row.created_at,
        }
    }
}

pub(crate) fn describe_incident(
    row: incident::Model,
    entries: Option<Vec<incident_entry::Model>>,
) -> IncidentResponse {
    let overdue = row.authority_notified_at.is_none()
        && matches!(row.status.as_str(), "open" | "contained")
        && row.report_due_at < Timestamp::now();
    IncidentResponse {
        id: row.id,
        data_categories: serde_json::from_str(&row.data_categories).unwrap_or_default(),
        title: row.title,
        description: row.description,
        severity: row.severity,
        status: row.status,
        detected_at: row.detected_at,
        report_due_at: row.report_due_at,
        overdue,
        authority_notified_at: row.authority_notified_at,
        authority_reference: row.authority_reference,
        subjects_notified_at: row.subjects_notified_at,
        affected_count: row.affected_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
        entries: entries.map(|items| items.into_iter().map(IncidentEntryResponse::from).collect()),
    }
}

/// Which records.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DeadlineQuery {
    /// Only records with this status.
    status: Option<String>,
    /// Only records past their deadline and not done.
    #[serde(default)]
    overdue: bool,
}

/// A personal data breach to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceIncidentRequest)]
pub(crate) struct IncidentRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// `low`, `medium`, `high` or `critical`.
    pub severity: String,
    /// When it was detected (not in the future); defaults to now.
    pub detected_at: Option<Timestamp>,
    #[serde(default)]
    pub data_categories: Vec<String>,
    pub affected_count: Option<i64>,
}

/// Changes to a breach (absent fields stay as they are).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceIncidentChange)]
pub(crate) struct IncidentChangeRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub severity: Option<String>,
    /// `open`, `contained`, `reported` (sets `authority_notified_at` to now if absent) or `closed`.
    pub status: Option<String>,
    pub data_categories: Option<Vec<String>>,
    pub affected_count: Option<i64>,
    /// When the authority was notified (moves an open breach to `reported`).
    pub authority_notified_at: Option<Timestamp>,
    /// Reference of the report at the authority.
    pub authority_reference: Option<String>,
    /// When the people affected were told.
    pub subjects_notified_at: Option<Timestamp>,
}

/// A line to add to the log.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceIncidentEntryRequest)]
pub(crate) struct EntryRequest {
    pub body: String,
}

/// List personal data breaches.
///
/// Requires `compliance.incidents.manage`.
#[utoipa::path(
    get,
    path = "/incidents",
    extensions(("x-permissions" = json!(["compliance.incidents.manage"]))),
    operation_id = "list_compliance_incidents",
    security(("bearer" = [])),
    params(DeadlineQuery, PageQuery),
    responses(
        (status = 200, description = "One page of breaches.", body = Page<IncidentResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.incidents.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_incidents(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<DeadlineQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<IncidentResponse>, Error> {
    principal.require(&crate::permissions::INCIDENTS_MANAGE)?;
    let page = service
        .list_incidents(
            &tenant,
            &principal,
            query.status.as_deref(),
            query.overdue,
            &PageRequest::try_from(page)?,
        )
        .await?;
    Ok(page.map(|row| describe_incident(row, None)))
}

/// Record a personal data breach.
///
/// Requires `compliance.incidents.manage`. The report to the authority is due 72 hours after
/// detection (legal rule `privacy.incident`; Personal Data Protection Law 91/2025/QH15, Decree
/// 356/2025/ND-CP). Reminders go to the `incident.alert_emails` policy; see the runbook in
/// docs/COMPLIANCE.md.
#[utoipa::path(
    post,
    path = "/incidents",
    extensions(("x-permissions" = json!(["compliance.incidents.manage"]))),
    operation_id = "open_compliance_incident",
    security(("bearer" = [])),
    request_body = IncidentRequest,
    responses(
        (status = 201, description = "The breach.", body = Data<IncidentResponse>, headers(("Location" = String, description = "Path of the breach."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.incidents.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn open_incident(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<IncidentRequest>,
) -> Result<Created<IncidentResponse>, Error> {
    let row = service
        .open_incident(
            &tenant,
            &principal,
            NewIncident {
                title: input.title,
                description: input.description,
                severity: input.severity,
                detected_at: input.detected_at,
                data_categories: input.data_categories,
                affected_count: input.affected_count,
            },
        )
        .await?;
    Ok(Created::new(
        format!("/compliance/incidents/{}", row.id),
        describe_incident(row, Some(Vec::new())),
    ))
}

/// Get a personal data breach with its log.
///
/// Requires `compliance.incidents.manage`.
#[utoipa::path(
    get,
    path = "/incidents/{id}",
    extensions(("x-permissions" = json!(["compliance.incidents.manage"]))),
    operation_id = "get_compliance_incident",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the breach.")),
    responses(
        (status = 200, description = "The breach.", body = Data<IncidentResponse>, headers(("ETag" = String, description = "Version; send it in `If-Match` to change it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.incidents.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such breach; code `compliance.incident_not_found`.", body = Problem),
    )
)]
async fn get_incident(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<IncidentResponse>, Error> {
    let (row, entries) = service.incident(&tenant, &principal, id).await?;
    Ok(Tagged { version: row.updated_at, data: describe_incident(row, Some(entries)) })
}

/// Change a personal data breach.
///
/// Requires `compliance.incidents.manage`. Send `If-Match` with the `ETag` you read. Record the
/// report with `authority_notified_at` and `authority_reference`.
#[utoipa::path(
    patch,
    path = "/incidents/{id}",
    extensions(("x-permissions" = json!(["compliance.incidents.manage"]))),
    operation_id = "update_compliance_incident",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the breach."), ("If-Match" = Option<String>, Header, description = "`ETag` of the version you read.")),
    request_body = IncidentChangeRequest,
    responses(
        (status = 200, description = "The breach after the change.", body = Data<IncidentResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.incidents.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such breach; code `compliance.incident_not_found`.", body = Problem),
        (status = 412, description = "Changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn update_incident(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    if_match: IfMatch,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<IncidentChangeRequest>,
) -> Result<Tagged<IncidentResponse>, Error> {
    principal.require(&crate::permissions::INCIDENTS_MANAGE)?;
    let change = IncidentChange {
        title: input.title,
        description: input.description,
        severity: input.severity,
        status: input.status,
        data_categories: input.data_categories,
        affected_count: input.affected_count,
        authority_notified_at: input.authority_notified_at,
        authority_reference: input.authority_reference,
        subjects_notified_at: input.subjects_notified_at,
    };
    let row =
        service.update_incident(&tenant, &principal, id, if_match.expected()?, change).await?;
    Ok(Tagged { version: row.updated_at, data: describe_incident(row, None) })
}

/// Add a line to the log of a personal data breach.
///
/// Requires `compliance.incidents.manage`. The log only grows.
#[utoipa::path(
    post,
    path = "/incidents/{id}/entries",
    extensions(("x-permissions" = json!(["compliance.incidents.manage"]))),
    operation_id = "add_compliance_incident_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the breach.")),
    request_body = EntryRequest,
    responses(
        (status = 201, description = "The line.", body = Data<IncidentEntryResponse>, headers(("Location" = String, description = "Path of the breach."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.incidents.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such breach; code `compliance.incident_not_found`.", body = Problem),
        (status = 422, description = "Empty or too long; every problem in `errors`.", body = Problem),
    )
)]
async fn add_incident_entry(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<EntryRequest>,
) -> Result<Created<IncidentEntryResponse>, Error> {
    let row = service.add_incident_entry(&tenant, &principal, id, &input.body).await?;
    Ok(Created::new(format!("/compliance/incidents/{id}"), row.into()))
}

/// Which draft.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DraftQuery {
    /// `authority` (notification to the data protection authority) or `subjects` (the people
    /// affected).
    audience: String,
    /// `vi` or `en` (default).
    locale: Option<String>,
}

/// A draft notification.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceIncidentDraft)]
pub(crate) struct DraftResponse {
    pub audience: String,
    pub locale: String,
    pub subject: String,
    pub text: String,
    /// Reminder that a legal professional must review it.
    pub disclaimer: String,
}

/// Get a draft notification of a personal data breach.
///
/// Requires `compliance.incidents.manage`. Filled from the breach and the tenant's `privacy.contact`
/// policy; a draft only — a person completes, reviews and sends it (the official form of the
/// authority needs a legal professional).
#[utoipa::path(
    get,
    path = "/incidents/{id}/draft",
    extensions(("x-permissions" = json!(["compliance.incidents.manage"]))),
    operation_id = "get_compliance_incident_draft",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the breach."), DraftQuery),
    responses(
        (status = 200, description = "The draft.", body = Data<DraftResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.incidents.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such breach; code `compliance.incident_not_found`.", body = Problem),
        (status = 422, description = "`audience` is not `authority` or `subjects`; every problem in `errors`.", body = Problem),
    )
)]
async fn incident_draft(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<DraftQuery>,
) -> Result<Data<DraftResponse>, Error> {
    let language = query.locale.as_deref().and_then(Language::from_tag).unwrap_or(Language::En);
    let (subject, text) =
        service.incident_draft(&tenant, &principal, id, &query.audience, language).await?;
    Ok(Data {
        data: DraftResponse {
            audience: query.audience,
            locale: language.tag().to_owned(),
            subject,
            text,
            disclaimer: crate::privacy::DISCLAIMER.to_owned(),
        },
    })
}

// ---- Yêu cầu của chủ thể ---------------------------------------------------------------------

/// A request of a data subject received through another channel.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceSubjectRequest)]
pub(crate) struct SubjectRequestResponse {
    pub id: Uuid,
    /// `access`, `erasure`, `rectification`, `objection`, `restriction` or `withdrawal`.
    pub kind: String,
    /// `email`, `post`, `phone`, `authority` or `other`.
    pub channel: String,
    /// `user`, `email` or `phone`.
    pub subject_kind: String,
    /// The user id, email address or phone number (E.164) of the person.
    pub subject: String,
    pub received_at: Timestamp,
    /// Answer (acknowledge) before this time (legal rule `privacy.subject_requests`, `respond_within_hours`).
    pub due_at: Timestamp,
    /// Carry the request out before this time (legal rule `privacy.subject_requests`, `fulfil_within_days` of the
    /// kind, in calendar days unless the rule says `working_days`); `null` for a request recorded before deadlines
    /// to fulfil were kept, or when the rule sets none for the kind.
    pub fulfil_due_at: Option<Timestamp>,
    /// `true` while still `received` past `due_at`, or not done (`received`, `in_progress`) past
    /// `fulfil_due_at`.
    pub overdue: bool,
    /// `received`, `in_progress`, `fulfilled` or `rejected`.
    pub status: String,
    pub reason: Option<String>,
    pub notes: Option<String>,
    pub handled_by: Option<Uuid>,
    pub closed_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub(crate) fn describe_request(row: subject_request::Model) -> SubjectRequestResponse {
    let overdue = crate::subject_requests::is_overdue(&row, Timestamp::now());
    SubjectRequestResponse {
        id: row.id,
        kind: row.kind,
        channel: row.channel,
        subject_kind: row.subject_kind,
        subject: row.subject,
        received_at: row.received_at,
        due_at: row.due_at,
        fulfil_due_at: row.fulfil_due_at,
        overdue,
        status: row.status,
        reason: row.reason,
        notes: row.notes,
        handled_by: row.handled_by,
        closed_at: row.closed_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// A request to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceSubjectRequestRequest)]
pub(crate) struct SubjectRequestRequest {
    /// `access`, `erasure`, `rectification`, `objection`, `restriction` or `withdrawal`.
    pub kind: String,
    /// `email`, `post`, `phone`, `authority` or `other`.
    pub channel: String,
    /// The person: a user id, an email address or a phone number.
    pub subject: String,
    /// When it was received (not in the future); defaults to now. The due date counts from here.
    pub received_at: Option<Timestamp>,
    pub notes: Option<String>,
}

/// A status change.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceSubjectRequestChange)]
pub(crate) struct SubjectRequestChange {
    /// `in_progress` or `rejected` (needs `reason`).
    pub status: Option<String>,
    pub reason: Option<String>,
    pub notes: Option<String>,
}

/// How a request was fulfilled.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceSubjectRequestFulfil)]
pub(crate) struct FulfilRequest {
    pub notes: Option<String>,
}

/// The result of fulfilling a request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceSubjectRequestResult)]
pub(crate) struct FulfilResponse {
    pub request: SubjectRequestResponse,
    /// For `access`: the person's data by module — send it to them; it is not stored.
    #[schema(value_type = Option<Object>)]
    pub export: Option<serde_json::Value>,
    /// For `erasure` / `withdrawal`: records erased or emptied.
    pub erased: Option<u64>,
    /// What a person still has to do, if anything.
    pub next_steps: Option<String>,
}

/// List the requests of data subjects received through other channels.
///
/// Requires `compliance.privacy.manage`.
#[utoipa::path(
    get,
    path = "/subject-requests",
    extensions(("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "list_compliance_subject_requests",
    security(("bearer" = [])),
    params(DeadlineQuery, PageQuery),
    responses(
        (status = 200, description = "One page of requests.", body = Page<SubjectRequestResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_subject_requests(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<DeadlineQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<SubjectRequestResponse>, Error> {
    principal.require(&crate::permissions::PRIVACY_MANAGE)?;
    let page = service
        .list_subject_requests(
            &tenant,
            &principal,
            query.status.as_deref(),
            query.overdue,
            &PageRequest::try_from(page)?,
        )
        .await?;
    Ok(page.map(describe_request))
}

/// Record a request of a data subject received by email, post, phone or from an authority.
///
/// Requires `compliance.privacy.manage`. The due date comes from the legal rule
/// `privacy.subject_requests` in force when it was received (Personal Data Protection Law
/// 91/2025/QH15, Decree 356/2025/ND-CP — have a legal professional check it). Requests a user makes
/// in their account run at once and are not recorded here.
#[utoipa::path(
    post,
    path = "/subject-requests",
    extensions(("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "record_compliance_subject_request",
    security(("bearer" = [])),
    request_body = SubjectRequestRequest,
    responses(
        (status = 201, description = "The request.", body = Data<SubjectRequestResponse>, headers(("Location" = String, description = "Path of the request."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn record_subject_request(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SubjectRequestRequest>,
) -> Result<Created<SubjectRequestResponse>, Error> {
    let row = service
        .record_subject_request(
            &tenant,
            &principal,
            NewRequest {
                kind: input.kind,
                channel: input.channel,
                subject: input.subject,
                received_at: input.received_at,
                notes: input.notes,
            },
        )
        .await?;
    Ok(Created::new(format!("/compliance/subject-requests/{}", row.id), describe_request(row)))
}

/// Get a request of a data subject.
///
/// Requires `compliance.privacy.manage`.
#[utoipa::path(
    get,
    path = "/subject-requests/{id}",
    extensions(("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "get_compliance_subject_request",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    responses(
        (status = 200, description = "The request.", body = Data<SubjectRequestResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such request; code `compliance.subject_request_not_found`.", body = Problem),
    )
)]
async fn get_subject_request(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SubjectRequestResponse>, Error> {
    Ok(Data { data: describe_request(service.subject_request(&tenant, &principal, id).await?) })
}

/// Change the status of a request of a data subject, or add notes.
///
/// Requires `compliance.privacy.manage`. Reject with a `reason`; fulfil with `POST …/fulfil`.
#[utoipa::path(
    patch,
    path = "/subject-requests/{id}",
    extensions(("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "update_compliance_subject_request",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    request_body = SubjectRequestChange,
    responses(
        (status = 200, description = "The request after the change.", body = Data<SubjectRequestResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such request; code `compliance.subject_request_not_found`.", body = Problem),
        (status = 409, description = "Already fulfilled or rejected; code `compliance.subject_request_closed`.", body = Problem),
        (status = 412, description = "Changed at the same time by someone else; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn update_subject_request(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<SubjectRequestChange>,
) -> Result<Data<SubjectRequestResponse>, Error> {
    let row = service
        .update_subject_request(&tenant, &principal, id, input.status, input.reason, input.notes)
        .await?;
    Ok(Data { data: describe_request(row) })
}

/// Fulfil a request of a data subject.
///
/// Requires `compliance.privacy.manage`. `access` of a user returns the personal data every module
/// keeps about them (send it to the person; it is not stored); `erasure` of an email address or
/// phone number empties it from the consent ledger; erasing a whole account is
/// `DELETE /api/v1/auth/users/{id}`. Other kinds are done by hand and only recorded.
///
/// An AI agent's call waits for a person's approval (202, see `x-approval`).
#[utoipa::path(
    post,
    path = "/subject-requests/{id}/fulfil",
    extensions(("x-approval" = json!("agent")), ("x-permissions" = json!(["compliance.privacy.manage"]))),
    operation_id = "fulfil_compliance_subject_request",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    request_body = FulfilRequest,
    responses(
        (status = 200, description = "What was done.", body = Data<FulfilResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.privacy.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such request; code `compliance.subject_request_not_found`.", body = Problem),
        (status = 409, description = "Already fulfilled or rejected; code `compliance.subject_request_closed`.", body = Problem),
        (status = 412, description = "Changed at the same time by someone else; code `precondition_failed`.", body = Problem),
        (status = 422, description = "Notes too long; every problem in `errors`.", body = Problem),
    )
)]
async fn fulfil_subject_request(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<FulfilRequest>,
) -> Result<Data<FulfilResponse>, Error> {
    let done = service.fulfil_subject_request(&tenant, &principal, id, input.notes).await?;
    Ok(Data {
        data: FulfilResponse {
            request: describe_request(done.request),
            export: done.export,
            erased: done.erased,
            next_steps: done.next_steps,
        },
    })
}
