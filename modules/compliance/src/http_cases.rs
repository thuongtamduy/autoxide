//! Route REST của vụ việc (D-363) và bảo vệ người tiêu dùng (D-364) dưới `/api/v1/compliance`: báo vi
//! phạm sở hữu trí tuệ / hàng giả, khiếu nại, phản hồi của bên bị phản ánh, quyết định, cung cấp thông
//! tin cho cơ quan, lý do được ẩn đánh giá.

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::cases::{CaseDetail, CaseFilter, NewCase, PublicCase, Reporter, reference};
use crate::disclosures::NewDisclosure;
use crate::entity::{case, case_entry, disclosure};
use crate::permissions::{CASES_READ, DISCLOSURES_MANAGE};
use crate::service::ComplianceService;

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_cases, open_case))
        .routes(routes!(get_case))
        .routes(routes!(add_case_entry))
        .routes(routes!(restrict_case))
        .routes(routes!(restore_case))
        .routes(routes!(decide_case))
        .routes(routes!(close_case))
        .routes(routes!(respondent_cases))
        .routes(routes!(respond_case))
        .routes(routes!(list_disclosures, record_disclosure))
        .routes(routes!(public_report))
        .routes(routes!(complaint_channel, public_complaint))
        .routes(routes!(tracked_case))
        .routes(routes!(moderation_reasons))
        .with_state(service)
}

// ---- Hình dạng trả về -------------------------------------------------------------------------

/// A line of the log of a case.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceCaseEntry)]
pub(crate) struct CaseEntryResponse {
    pub id: Uuid,
    /// `note` (internal), `evidence`, `status`, `restriction`, `response` (of the reported
    /// party), `decision` or `disclosure`.
    pub kind: String,
    /// `staff`, `respondent` or `system`.
    pub author_kind: String,
    pub author_user_id: Option<Uuid>,
    pub body: String,
    /// Ids of files (module `files`).
    pub files: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

impl From<case_entry::Model> for CaseEntryResponse {
    fn from(row: case_entry::Model) -> Self {
        CaseEntryResponse {
            id: row.id,
            files: serde_json::from_str(&row.files).unwrap_or_default(),
            kind: row.kind,
            author_kind: row.author_kind,
            author_user_id: row.author_user_id,
            body: row.body,
            created_at: row.created_at,
        }
    }
}

/// A report of an infringement (intellectual property, counterfeit, prohibited goods) or a
/// consumer complaint, as the people who handle it see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceCase)]
pub(crate) struct CaseResponse {
    pub id: Uuid,
    /// Short reference for people, e.g. `C-1A2B3C4D`.
    pub reference: String,
    /// `ip_infringement`, `counterfeit`, `prohibited_goods`, `consumer_complaint` or `other`.
    pub kind: String,
    /// The reported object, `<module>.<type>:<id>` (e.g. `content.entry:…`); empty when none.
    pub target: String,
    /// The reported unit (seller), when given.
    pub respondent_org_id: Option<Uuid>,
    /// Users who may answer: the owners of the target.
    pub respondent_user_ids: Vec<Uuid>,
    /// `guest`, `user`, `rights_holder` or `authority`.
    pub reporter_kind: String,
    pub reporter_user_id: Option<Uuid>,
    pub reporter_name: Option<String>,
    pub reporter_email: Option<String>,
    pub reporter_phone: Option<String>,
    /// `public` (the public form) or `staff` (entered by staff).
    pub channel: String,
    pub claim: String,
    /// Certificates, registrations, links given as evidence.
    #[schema(value_type = Vec<Object>)]
    pub rights: Vec<Value>,
    /// `received` (held by the anti-spam checks), `under_review`, `awaiting_response`, `decided`,
    /// `closed` or `withdrawn`.
    pub status: String,
    /// The target is hidden while the case is handled (or after an upheld decision).
    pub restricted: bool,
    /// `upheld`, `rejected`, `partially_upheld`, or empty before the decision.
    pub decision: String,
    /// Reason of the decision; shown to the reporter and the reported party.
    pub decision_reason: Option<String>,
    /// Acknowledge the case before this time (legal rule `cases.deadlines`).
    pub ack_due_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    /// The reported party may answer until this time.
    pub respond_by: DateTime<Utc>,
    /// Decide before this time (working days of the legal rule `cases.deadlines`; 24 hours for
    /// a request of an authority).
    pub decide_by: DateTime<Utc>,
    /// `true` while open and past `decide_by`.
    pub overdue: bool,
    pub decided_at: Option<DateTime<Utc>>,
    pub decided_by: Option<Uuid>,
    /// Why the anti-spam checks held it; `null` when not held.
    pub held_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// The tracking code for the reporter — only in the answer that created the case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_token: Option<String>,
    /// The log, oldest first (only when reading one case).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<Vec<CaseEntryResponse>>,
}

fn is_open(status: &str) -> bool {
    matches!(status, "received" | "under_review" | "awaiting_response")
}

pub(crate) fn describe_case(
    row: case::Model,
    respondents: Vec<Uuid>,
    entries: Option<Vec<case_entry::Model>>,
) -> CaseResponse {
    CaseResponse {
        reference: reference(row.id),
        overdue: is_open(&row.status) && row.decide_by < Utc::now(),
        rights: serde_json::from_str(&row.rights).unwrap_or_default(),
        id: row.id,
        kind: row.kind,
        target: row.target,
        respondent_org_id: row.respondent_org_id,
        respondent_user_ids: respondents,
        reporter_kind: row.reporter_kind,
        reporter_user_id: row.reporter_user_id,
        reporter_name: row.reporter_name,
        reporter_email: row.reporter_email,
        reporter_phone: row.reporter_phone,
        channel: row.channel,
        claim: row.claim,
        status: row.status,
        restricted: row.restricted,
        decision: row.decision,
        decision_reason: row.decision_reason,
        ack_due_at: row.ack_due_at,
        acknowledged_at: row.acknowledged_at,
        respond_by: row.respond_by,
        decide_by: row.decide_by,
        decided_at: row.decided_at,
        decided_by: row.decided_by,
        held_reason: row.spam_reason,
        created_at: row.created_at,
        updated_at: row.updated_at,
        tracking_token: None,
        entries: entries.map(|items| items.into_iter().map(CaseEntryResponse::from).collect()),
    }
}

pub(crate) fn describe_detail(detail: CaseDetail) -> CaseResponse {
    describe_case(detail.case, detail.respondents, Some(detail.entries))
}

/// A case about you (or your unit), as the reported party sees it: no contact details of the
/// reporter, no internal notes.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceRespondentCase)]
pub(crate) struct RespondentCaseResponse {
    pub id: Uuid,
    pub reference: String,
    pub kind: String,
    pub target: String,
    pub respondent_org_id: Option<Uuid>,
    pub reporter_kind: String,
    /// Name of the reporter when it is a rights holder or an authority; `null` otherwise.
    pub reporter_name: Option<String>,
    pub claim: String,
    #[schema(value_type = Vec<Object>)]
    pub rights: Vec<Value>,
    pub status: String,
    pub restricted: bool,
    pub decision: String,
    pub decision_reason: Option<String>,
    /// Answer with `POST /cases/{id}/responses` before this time.
    pub respond_by: DateTime<Utc>,
    pub decide_by: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    /// Status changes, restrictions, answers and the decision, oldest first.
    pub entries: Vec<CaseEntryResponse>,
}

fn describe_for_respondent(detail: CaseDetail) -> RespondentCaseResponse {
    let row = detail.case;
    let named = matches!(row.reporter_kind.as_str(), "rights_holder" | "authority");
    RespondentCaseResponse {
        reference: reference(row.id),
        rights: serde_json::from_str(&row.rights).unwrap_or_default(),
        reporter_name: if named { row.reporter_name } else { None },
        entries: detail
            .entries
            .into_iter()
            .filter(|item| {
                matches!(item.kind.as_str(), "status" | "restriction" | "response" | "decision")
            })
            .map(CaseEntryResponse::from)
            .collect(),
        id: row.id,
        kind: row.kind,
        target: row.target,
        respondent_org_id: row.respondent_org_id,
        reporter_kind: row.reporter_kind,
        claim: row.claim,
        status: row.status,
        restricted: row.restricted,
        decision: row.decision,
        decision_reason: row.decision_reason,
        respond_by: row.respond_by,
        decide_by: row.decide_by,
        created_at: row.created_at,
    }
}

/// A step of a case, as the reporter sees it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceCaseStep)]
pub(crate) struct CaseStep {
    /// `status`, `restriction` or `decision`.
    pub kind: String,
    pub at: DateTime<Utc>,
}

/// The state of a report or complaint, for the person who sent it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceTrackedCase)]
pub(crate) struct TrackedCaseResponse {
    pub id: Uuid,
    pub reference: String,
    pub kind: String,
    pub target: String,
    pub status: String,
    pub restricted: bool,
    pub decision: String,
    pub decision_reason: Option<String>,
    pub ack_due_at: DateTime<Utc>,
    pub acknowledged_at: Option<DateTime<Utc>>,
    pub decide_by: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub steps: Vec<CaseStep>,
}

fn describe_tracked(detail: CaseDetail) -> TrackedCaseResponse {
    let row = detail.case;
    TrackedCaseResponse {
        reference: reference(row.id),
        steps: detail
            .entries
            .into_iter()
            .filter(|item| matches!(item.kind.as_str(), "status" | "restriction" | "decision"))
            .map(|item| CaseStep { kind: item.kind, at: item.created_at })
            .collect(),
        id: row.id,
        kind: row.kind,
        target: row.target,
        // Vụ lớp chống spam giữ lại hiện như vụ đang xem xét: người gửi không biết bị giữ.
        status: if row.status == "received" { "under_review".to_owned() } else { row.status },
        restricted: row.restricted,
        decision: row.decision,
        decision_reason: row.decision_reason,
        ack_due_at: row.ack_due_at,
        acknowledged_at: row.acknowledged_at,
        decide_by: row.decide_by,
        decided_at: row.decided_at,
        created_at: row.created_at,
    }
}

// ---- Nhân sự xử lý -------------------------------------------------------------------------------

/// Which cases.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CaseQuery {
    /// Only cases with this status.
    status: Option<String>,
    /// Only cases of this kind.
    kind: Option<String>,
    /// Only cases about this object (`<module>.<type>:<id>`).
    target: Option<String>,
    /// Only open cases past `decide_by`.
    #[serde(default)]
    overdue: bool,
    /// `true`: only cases the anti-spam checks held (status `received`); default: the others.
    #[serde(default)]
    held: bool,
}

/// A case to record (e.g. a letter of an authority or of a rights holder).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCaseRequest)]
pub(crate) struct CaseRequest {
    /// `ip_infringement`, `counterfeit`, `prohibited_goods`, `consumer_complaint` or `other`.
    pub kind: String,
    /// The reported object, `<module>.<type>:<id>` (e.g. `content.entry:…`); required except
    /// for `consumer_complaint` and `other`.
    pub target: Option<String>,
    /// The reported unit (seller), if known.
    pub respondent_org_id: Option<Uuid>,
    /// `guest`, `user`, `rights_holder` or `authority` (a request of a competent authority:
    /// the target is hidden at once and the decision is due in the hours of the rule).
    pub reporter_kind: String,
    pub reporter_user_id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    /// What is reported, 10 to 20000 characters.
    pub claim: String,
    /// Certificates, registrations, links (objects; at most 20).
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub rights: Vec<Value>,
}

/// List cases.
///
/// Requires `compliance.cases.read`.
#[utoipa::path(
    get,
    path = "/cases",
    extensions(("x-permissions" = json!(["compliance.cases.read"]))),
    operation_id = "list_compliance_cases",
    security(("bearer" = [])),
    params(CaseQuery, PageQuery),
    responses(
        (status = 200, description = "One page of cases.", body = Page<CaseResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_cases(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CaseQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<CaseResponse>, Error> {
    principal.require(&CASES_READ)?;
    let filter = CaseFilter {
        status: query.status,
        kind: query.kind,
        target: query.target,
        overdue: query.overdue,
        held: query.held,
    };
    let page =
        service.list_cases(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(|row| describe_case(row, Vec::new(), None)))
}

/// Record a case received outside the public forms.
///
/// Requires `compliance.cases.handle`. Deadlines come from the legal rule `cases.deadlines`. A
/// request of a competent authority hides the target at once; so does an intellectual property
/// or counterfeit report when the tenant's `cases.auto_restrict` policy is `true`. The answer
/// carries `tracking_token` once: give it to the reporter.
#[utoipa::path(
    post,
    path = "/cases",
    extensions(("x-permissions" = json!(["compliance.cases.handle"]))),
    operation_id = "open_compliance_case",
    security(("bearer" = [])),
    request_body = CaseRequest,
    responses(
        (status = 201, description = "The case, with `tracking_token`.", body = Data<CaseResponse>, headers(("Location" = String, description = "Path of the case."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.handle`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid or the target does not exist; every problem in `errors`.", body = Problem),
    )
)]
async fn open_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CaseRequest>,
) -> Result<Created<CaseResponse>, Error> {
    let opened = service
        .open_case(
            &tenant,
            &principal,
            NewCase {
                kind: input.kind,
                target: input.target,
                respondent_org_id: input.respondent_org_id,
                reporter: Reporter {
                    kind: input.reporter_kind,
                    user_id: input.reporter_user_id,
                    name: input.name,
                    email: input.email,
                    phone: input.phone,
                },
                claim: input.claim,
                rights: input.rights,
            },
        )
        .await?;
    let detail = service.case_after(&tenant, opened.case.id).await?;
    let mut data = describe_detail(detail);
    data.tracking_token = Some(opened.token);
    Ok(Created::new(format!("/compliance/cases/{}", data.id), data))
}

/// Get a case with its log.
///
/// Requires `compliance.cases.read`.
#[utoipa::path(
    get,
    path = "/cases/{id}",
    extensions(("x-permissions" = json!(["compliance.cases.read"]))),
    operation_id = "get_compliance_case",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the case.")),
    responses(
        (status = 200, description = "The case.", body = Data<CaseResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such case; code `compliance.case_not_found`.", body = Problem),
    )
)]
async fn get_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CaseResponse>, Error> {
    let detail = service.case_detail(&tenant, &principal, id).await?;
    Ok(Data { data: describe_detail(detail) })
}

/// A line to add to the log of a case.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCaseEntryRequest)]
pub(crate) struct CaseEntryRequest {
    /// `note` (internal, the default) or `evidence`.
    #[serde(default = "note")]
    pub kind: String,
    pub body: String,
    /// Ids of files uploaded to the module `files`.
    #[serde(default)]
    pub files: Vec<Uuid>,
}

fn note() -> String {
    "note".to_owned()
}

/// Add a note or evidence to a case.
///
/// Requires `compliance.cases.handle`. The log only grows. The first action on a new case
/// records that it was acknowledged; on a case held by the anti-spam checks it releases it.
#[utoipa::path(
    post,
    path = "/cases/{id}/entries",
    extensions(("x-permissions" = json!(["compliance.cases.handle"]))),
    operation_id = "add_compliance_case_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the case.")),
    request_body = CaseEntryRequest,
    responses(
        (status = 201, description = "The line.", body = Data<CaseEntryResponse>, headers(("Location" = String, description = "Path of the case."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.handle`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such case; code `compliance.case_not_found`.", body = Problem),
        (status = 412, description = "The case changed meanwhile; code `precondition_failed`: try again.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn add_case_entry(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CaseEntryRequest>,
) -> Result<Created<CaseEntryResponse>, Error> {
    let row = service
        .add_case_entry(&tenant, &principal, id, &input.kind, &input.body, &input.files)
        .await?;
    Ok(Created::new(format!("/compliance/cases/{id}"), row.into()))
}

/// Why.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCaseReason)]
pub(crate) struct ReasonRequest {
    /// 1 to 20000 characters; kept in the log.
    pub reason: String,
}

/// Hide the target of a case while it is handled.
///
/// Requires `compliance.cases.handle`. Sends the event `compliance.target_restricted`: the module
/// that owns the target hides it (e.g. a content page answers 451, a comment is hidden). Already
/// hidden: nothing changes.
#[utoipa::path(
    post,
    path = "/cases/{id}/restrict",
    extensions(("x-permissions" = json!(["compliance.cases.handle"]))),
    operation_id = "restrict_compliance_case_target",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the case.")),
    request_body = ReasonRequest,
    responses(
        (status = 200, description = "The case.", body = Data<CaseResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.handle`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such case; code `compliance.case_not_found`.", body = Problem),
        (status = 409, description = "The case is not open or has no target; code `compliance.case_state_invalid`.", body = Problem),
        (status = 412, description = "The case changed meanwhile; code `precondition_failed`: try again.", body = Problem),
        (status = 422, description = "`reason` is empty or too long; every problem in `errors`.", body = Problem),
    )
)]
async fn restrict_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReasonRequest>,
) -> Result<Data<CaseResponse>, Error> {
    service.restrict_case_target(&tenant, &principal, id, &input.reason).await?;
    let detail = service.case_after(&tenant, id).await?;
    Ok(Data { data: describe_detail(detail) })
}

/// Show the target of a case again before the decision.
///
/// Requires `compliance.cases.handle`. Sends `compliance.target_restored` unless another case
/// still hides the same target.
#[utoipa::path(
    post,
    path = "/cases/{id}/restore",
    extensions(("x-permissions" = json!(["compliance.cases.handle"]))),
    operation_id = "restore_compliance_case_target",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the case.")),
    request_body = ReasonRequest,
    responses(
        (status = 200, description = "The case.", body = Data<CaseResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.handle`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such case; code `compliance.case_not_found`.", body = Problem),
        (status = 409, description = "The case is not open or its target is not hidden; code `compliance.case_state_invalid`.", body = Problem),
        (status = 412, description = "The case changed meanwhile; code `precondition_failed`: try again.", body = Problem),
        (status = 422, description = "`reason` is empty or too long; every problem in `errors`.", body = Problem),
    )
)]
async fn restore_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReasonRequest>,
) -> Result<Data<CaseResponse>, Error> {
    service.restore_case_target(&tenant, &principal, id, &input.reason).await?;
    let detail = service.case_after(&tenant, id).await?;
    Ok(Data { data: describe_detail(detail) })
}

/// The decision on a case.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCaseDecision)]
pub(crate) struct DecisionRequest {
    /// `upheld` (the target stays hidden, or is hidden now), `partially_upheld` (same) or
    /// `rejected` (the target is shown again).
    pub decision: String,
    /// Shown to the reporter and the reported party; 1 to 20000 characters.
    pub reason: String,
}

/// Decide a case.
///
/// Requires `compliance.cases.handle` and a signed-in person (API keys get 403
/// `compliance.session_required`). The reporter is told by mail when they gave an address.
#[utoipa::path(
    post,
    path = "/cases/{id}/decision",
    extensions(("x-permissions" = json!(["compliance.cases.handle"]))),
    operation_id = "decide_compliance_case",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the case.")),
    request_body = DecisionRequest,
    responses(
        (status = 200, description = "The case.", body = Data<CaseResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.handle` (code `permission_denied`) or not a signed-in person (code `compliance.session_required`).", body = Problem),
        (status = 404, description = "No such case; code `compliance.case_not_found`.", body = Problem),
        (status = 409, description = "The case is not open; code `compliance.case_state_invalid`.", body = Problem),
        (status = 412, description = "The case changed meanwhile; code `precondition_failed`: try again.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn decide_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<DecisionRequest>,
) -> Result<Data<CaseResponse>, Error> {
    service.decide_case(&tenant, &principal, id, &input.decision, &input.reason).await?;
    let detail = service.case_after(&tenant, id).await?;
    Ok(Data { data: describe_detail(detail) })
}

/// Closing a case.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCaseClose)]
pub(crate) struct CloseRequest {
    /// `true` when the reporter withdrew it (open cases only).
    #[serde(default)]
    pub withdrawn: bool,
    /// 1 to 20000 characters; kept in the log.
    pub reason: String,
}

/// Close a case (spam, duplicate, withdrawn, or done after the decision).
///
/// Requires `compliance.cases.handle`. A hidden target is shown again unless the decision was
/// `upheld` or `partially_upheld`.
#[utoipa::path(
    post,
    path = "/cases/{id}/close",
    extensions(("x-permissions" = json!(["compliance.cases.handle"]))),
    operation_id = "close_compliance_case",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the case.")),
    request_body = CloseRequest,
    responses(
        (status = 200, description = "The case.", body = Data<CaseResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.cases.handle`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such case; code `compliance.case_not_found`.", body = Problem),
        (status = 409, description = "The case is already closed; code `compliance.case_state_invalid`.", body = Problem),
        (status = 412, description = "The case changed meanwhile; code `precondition_failed`: try again.", body = Problem),
        (status = 422, description = "`reason` is empty or too long; every problem in `errors`.", body = Problem),
    )
)]
async fn close_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CloseRequest>,
) -> Result<Data<CaseResponse>, Error> {
    service.close_case(&tenant, &principal, id, input.withdrawn, &input.reason).await?;
    let detail = service.case_after(&tenant, id).await?;
    Ok(Data { data: describe_detail(detail) })
}

// ---- Bên bị phản ánh -----------------------------------------------------------------------------

/// List the cases about you or your unit.
///
/// Signed in. You are the reported party when you own the target (e.g. the author of the page)
/// or have `compliance.cases.respond` on the reported unit (a seller on a marketplace).
#[utoipa::path(
    get,
    path = "/respondent/cases",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_compliance_cases",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of cases.", body = Page<RespondentCaseResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn respondent_cases(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<RespondentCaseResponse>, Error> {
    let page = service.respondent_cases(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(describe_for_respondent))
}

/// An answer of the reported party.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceCaseResponseRequest)]
pub(crate) struct AnswerRequest {
    /// Your answer, 1 to 20000 characters.
    pub body: String,
    /// Ids of files (e.g. invoices, licences) uploaded to the module `files`.
    #[serde(default)]
    pub files: Vec<Uuid>,
}

/// Answer a case about you or your unit.
///
/// Signed in, as the reported party (see `GET /respondent/cases`), before `respond_by`.
#[utoipa::path(
    post,
    path = "/cases/{id}/responses",
    extensions(("x-permissions" = json!([]))),
    operation_id = "respond_compliance_case",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the case.")),
    request_body = AnswerRequest,
    responses(
        (status = 201, description = "Your answer.", body = Data<CaseEntryResponse>, headers(("Location" = String, description = "Path of your cases."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such case about you; code `compliance.case_not_found`.", body = Problem),
        (status = 409, description = "Past `respond_by`, or the case is decided; code `compliance.response_closed`.", body = Problem),
        (status = 412, description = "The case changed meanwhile; code `precondition_failed`: try again.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn respond_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AnswerRequest>,
) -> Result<Created<CaseEntryResponse>, Error> {
    let row = service.respond_case(&tenant, &principal, id, &input.body, &input.files).await?;
    Ok(Created::new("/compliance/respondent/cases".to_owned(), row.into()))
}

// ---- Cung cấp thông tin cho cơ quan --------------------------------------------------------------

/// A disclosure of information to a competent authority.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceDisclosure)]
pub(crate) struct DisclosureResponse {
    pub id: Uuid,
    pub case_id: Option<Uuid>,
    pub authority: String,
    /// Number of the authority's request.
    pub request_reference: String,
    pub legal_basis: String,
    pub requested_at: DateTime<Utc>,
    /// `org:<unit id>` or `user:<user id>`.
    pub subject: String,
    /// Names of the fields given (the values are not kept).
    pub fields: Vec<String>,
    pub disclosed_by: Uuid,
    pub disclosed_at: DateTime<Utc>,
    /// The values given — only in the answer that recorded the disclosure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Object>)]
    pub values: Option<BTreeMap<String, Value>>,
}

impl From<disclosure::Model> for DisclosureResponse {
    fn from(row: disclosure::Model) -> Self {
        DisclosureResponse {
            id: row.id,
            fields: serde_json::from_str(&row.fields).unwrap_or_default(),
            case_id: row.case_id,
            authority: row.authority,
            request_reference: row.request_reference,
            legal_basis: row.legal_basis,
            requested_at: row.requested_at,
            subject: row.subject,
            disclosed_by: row.disclosed_by,
            disclosed_at: row.disclosed_at,
            values: None,
        }
    }
}

/// Which disclosures.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DisclosureQuery {
    /// Only the disclosures of this case.
    case_id: Option<Uuid>,
}

/// A request of a competent authority for information about a seller.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceDisclosureRequest)]
pub(crate) struct DisclosureRequest {
    /// The case it belongs to, if any (a line is added to its log).
    pub case_id: Option<Uuid>,
    /// The authority, e.g. the market surveillance office of a province.
    pub authority: String,
    /// Number of the authority's written request.
    pub request_reference: String,
    /// The legal basis the authority cites.
    pub legal_basis: String,
    /// When the request was received; defaults to now.
    pub requested_at: Option<DateTime<Utc>>,
    /// `org:<unit id>` (a seller's legal profile and identity verification) or `user:<user id>`.
    pub subject: String,
    /// Fields to give. Units: `legal_name`, `trade_name`, `tax_code`, `registration_number`,
    /// `address`, `representative_name`, `phone`, `email`, `verified_name`. Users:
    /// `verified_name`, `document_kind`, `document_last4`, `kyc_status`, `phone`.
    pub fields: Vec<String>,
}

/// List disclosures to authorities.
///
/// Requires `compliance.disclosures.manage`.
#[utoipa::path(
    get,
    path = "/disclosures",
    extensions(("x-permissions" = json!(["compliance.disclosures.manage"]))),
    operation_id = "list_compliance_disclosures",
    security(("bearer" = [])),
    params(DisclosureQuery, PageQuery),
    responses(
        (status = 200, description = "One page of disclosures (field names, no values).", body = Page<DisclosureResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.disclosures.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_disclosures(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<DisclosureQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<DisclosureResponse>, Error> {
    principal.require(&DISCLOSURES_MANAGE)?;
    let page = service
        .list_disclosures(&tenant, &principal, query.case_id, &PageRequest::try_from(page)?)
        .await?;
    Ok(page.map(DisclosureResponse::from))
}

/// Give a competent authority information about a seller, and record it.
///
/// Requires `compliance.disclosures.manage` and a signed-in person (API keys get 403
/// `compliance.session_required`). The answer has the values once; the record keeps the
/// authority, its request, the legal basis and the names of the fields, and the event
/// `compliance.disclosure_recorded` goes to the audit log.
#[utoipa::path(
    post,
    path = "/disclosures",
    extensions(("x-permissions" = json!(["compliance.disclosures.manage"]))),
    operation_id = "record_compliance_disclosure",
    security(("bearer" = [])),
    request_body = DisclosureRequest,
    responses(
        (status = 201, description = "The record, with `values`.", body = Data<DisclosureResponse>, headers(("Location" = String, description = "Path of the list of disclosures."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.disclosures.manage` (code `permission_denied`) or not a signed-in person (code `compliance.session_required`).", body = Problem),
        (status = 404, description = "No such case; code `compliance.case_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn record_disclosure(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<DisclosureRequest>,
) -> Result<Created<DisclosureResponse>, Error> {
    let (row, values) = service
        .record_disclosure(
            &tenant,
            &principal,
            NewDisclosure {
                case_id: input.case_id,
                authority: input.authority,
                request_reference: input.request_reference,
                legal_basis: input.legal_basis,
                requested_at: input.requested_at,
                subject: input.subject,
                fields: input.fields,
            },
        )
        .await?;
    let mut data = DisclosureResponse::from(row);
    data.values = Some(values);
    Ok(Created::new("/compliance/disclosures".to_owned(), data))
}

// ---- Công khai -----------------------------------------------------------------------------------

/// A report of an infringement from the public form.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CompliancePublicReport)]
pub(crate) struct PublicReportRequest {
    /// `ip_infringement`, `counterfeit`, `prohibited_goods` or `other`.
    pub kind: String,
    /// The reported object, `<module>.<type>:<id>` (e.g. `content.entry:…`); required except for
    /// `other`.
    pub target: Option<String>,
    /// The reported unit (seller on a marketplace), if known.
    pub respondent_org_id: Option<Uuid>,
    /// `true` when you hold the right (or act for its holder).
    #[serde(default)]
    pub rights_holder: bool,
    /// Your name (required when not signed in).
    pub name: Option<String>,
    /// Your email address or phone number (one of them when not signed in) to be told the outcome.
    pub email: Option<String>,
    pub phone: Option<String>,
    /// What is wrong, 10 to 20000 characters.
    pub claim: String,
    /// Certificates, registrations, links (objects; at most 20).
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub rights: Vec<Value>,
    /// Ask `GET /api/v1/antispam/token?scope=compliance:reports` when the form is shown.
    pub form_token: Option<String>,
    /// The hidden trap field: people leave it empty.
    pub trap: Option<String>,
    /// The value of the `altcha` widget, when the server uses a challenge.
    pub challenge: Option<String>,
}

/// A consumer complaint from the public form.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CompliancePublicComplaint)]
pub(crate) struct PublicComplaintRequest {
    /// What the complaint is about, `<module>.<type>:<id>` (e.g. an order or a product), if any.
    pub target: Option<String>,
    /// The seller complained about (marketplace), if known.
    pub respondent_org_id: Option<Uuid>,
    /// Your name (required when not signed in).
    pub name: Option<String>,
    /// Your email address or phone number (one of them when not signed in).
    pub email: Option<String>,
    pub phone: Option<String>,
    /// The complaint, 10 to 20000 characters.
    pub claim: String,
    /// Ask `GET /api/v1/antispam/token?scope=compliance:complaints` when the form is shown.
    pub form_token: Option<String>,
    /// The hidden trap field: people leave it empty.
    pub trap: Option<String>,
    /// The value of the `altcha` widget, when the server uses a challenge.
    pub challenge: Option<String>,
}

/// What the sender of a report or complaint gets back.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CompliancePublicCaseReceipt)]
pub(crate) struct ReceiptResponse {
    pub id: Uuid,
    pub reference: String,
    /// Keep it: with `id` it shows the state at `GET /public/cases/{id}?token=`. Not shown again.
    pub tracking_token: String,
}

/// Send a report of an infringement (intellectual property, counterfeit, prohibited goods).
///
/// No sign-in (the tenant comes from the domain). Send `form_token` from `GET
/// /api/v1/antispam/token?scope=compliance:reports`; limited per network block and email per
/// hour. Signed-in senders are recorded as the reporter. The answer is the same whether the
/// report is kept or dropped by the anti-spam checks.
#[utoipa::path(
    post,
    path = "/public/reports",
    extensions(("x-public" = json!(true))),
    operation_id = "send_public_compliance_report",
    request_body = PublicReportRequest,
    responses(
        (status = 201, description = "Received; keep `tracking_token`.", body = Data<ReceiptResponse>, headers(("Location" = String, description = "Path to follow it."))),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 422, description = "A value is invalid or the target does not exist; every problem in `errors`.", body = Problem),
        (status = 429, description = "Too many reports; code `antispam.rate_limited`, wait `Retry-After` seconds.", body = Problem),
    )
)]
async fn public_report(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    client: ClientInfo,
    ValidJson(input): ValidJson<PublicReportRequest>,
) -> Result<Created<ReceiptResponse>, Error> {
    let case = NewCase {
        kind: input.kind,
        target: input.target,
        respondent_org_id: input.respondent_org_id,
        reporter: Reporter {
            kind: if input.rights_holder { "rights_holder" } else { "guest" }.to_owned(),
            name: input.name,
            email: input.email,
            phone: input.phone,
            ..Reporter::default()
        },
        claim: input.claim,
        rights: input.rights,
    };
    let input = PublicCase {
        case,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    receipt(service.submit_public_case(&tenant, principal.as_ref(), &client, false, input).await?)
}

fn receipt((id, token): (Uuid, String)) -> Result<Created<ReceiptResponse>, Error> {
    Ok(Created::new(
        format!("/compliance/public/cases/{id}"),
        ReceiptResponse { id, reference: reference(id), tracking_token: token },
    ))
}

/// Send a consumer complaint.
///
/// No sign-in (the tenant comes from the domain). Send `form_token` from `GET
/// /api/v1/antispam/token?scope=compliance:complaints`. Deadlines come from the legal rule
/// `cases.deadlines` (Consumer Protection Law 19/2023/QH15, Decree 55/2024/ND-CP).
#[utoipa::path(
    post,
    path = "/public/complaints",
    extensions(("x-public" = json!(true))),
    operation_id = "send_public_compliance_complaint",
    request_body = PublicComplaintRequest,
    responses(
        (status = 201, description = "Received; keep `tracking_token`.", body = Data<ReceiptResponse>, headers(("Location" = String, description = "Path to follow it."))),
        (status = 400, description = "The form token or challenge is missing, invalid or expired (codes `antispam.token_missing`, `antispam.token_invalid`, `antispam.token_expired`, `antispam.challenge_required`, `antispam.challenge_failed`): reload the form.", body = Problem),
        (status = 422, description = "A value is invalid or the target does not exist; every problem in `errors`.", body = Problem),
        (status = 429, description = "Too many complaints; code `antispam.rate_limited`, wait `Retry-After` seconds.", body = Problem),
    )
)]
async fn public_complaint(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    client: ClientInfo,
    ValidJson(input): ValidJson<PublicComplaintRequest>,
) -> Result<Created<ReceiptResponse>, Error> {
    let case = NewCase {
        kind: "consumer_complaint".to_owned(),
        target: input.target,
        respondent_org_id: input.respondent_org_id,
        reporter: Reporter {
            kind: "guest".to_owned(),
            name: input.name,
            email: input.email,
            phone: input.phone,
            ..Reporter::default()
        },
        claim: input.claim,
        rights: Vec::new(),
    };
    let input = PublicCase {
        case,
        form_token: input.form_token,
        trap: input.trap,
        challenge: input.challenge,
    };
    receipt(service.submit_public_case(&tenant, principal.as_ref(), &client, true, input).await?)
}

/// How consumers can complain, and how fast they get an answer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceComplaintChannel)]
pub(crate) struct ComplaintChannelResponse {
    /// The tenant's `complaints.contact` policy (e.g. email, phone, address, hours); `null` when
    /// not set.
    #[schema(value_type = Option<Object>)]
    pub contact: Option<Value>,
    /// Send complaints with `POST /api/v1/compliance/public/complaints`.
    pub submit_path: String,
    pub acknowledge_within_hours: i64,
    pub respond_within_working_days: i64,
    pub decide_within_working_days: i64,
    /// The general conditions of trade, when published:
    /// `GET /api/v1/compliance/public/documents/general_conditions`.
    pub general_conditions_path: String,
    /// The legal basis of the deadlines (to check against the original texts).
    pub legal_basis: Option<String>,
}

/// Get the consumer complaint channel.
///
/// No sign-in (the tenant comes from the domain). For the website's complaint page (Consumer
/// Protection Law 19/2023/QH15, Decree 55/2024/ND-CP).
#[utoipa::path(
    get,
    path = "/public/complaints",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_compliance_complaint_channel",
    responses(
        (status = 200, description = "The channel.", body = Data<ComplaintChannelResponse>),
    )
)]
async fn complaint_channel(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
) -> Result<Data<ComplaintChannelResponse>, Error> {
    Ok(Data { data: service.complaint_channel(&tenant).await? })
}

/// The tracking code.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TrackingQuery {
    /// The `tracking_token` given when the case was sent.
    #[serde(default)]
    token: String,
}

/// Follow a report or complaint you sent.
///
/// No sign-in: the tracking code is the permission. A wrong code answers 404, like a case that
/// does not exist.
#[utoipa::path(
    get,
    path = "/public/cases/{id}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_compliance_case",
    params(("id" = Uuid, Path, description = "Id of the case."), TrackingQuery),
    responses(
        (status = 200, description = "The state of the case.", body = Data<TrackedCaseResponse>),
        (status = 404, description = "No such case, or wrong code; code `compliance.case_not_found`.", body = Problem),
    )
)]
async fn tracked_case(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    Query(query): Query<TrackingQuery>,
) -> Result<Data<TrackedCaseResponse>, Error> {
    let detail = service.tracked_case(&tenant, id, &query.token).await?;
    Ok(Data { data: describe_tracked(detail) })
}

/// Which surface.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SurfaceQuery {
    /// Where content is moderated, e.g. `reviews` or `comments`; default `reviews`.
    surface: Option<String>,
}

/// The reasons for which content of consumers may be hidden.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceModerationReasons)]
pub(crate) struct ModerationReasonsResponse {
    pub surface: String,
    /// Reason codes, e.g. `spam`, `illegal_content`, `personal_data`, `unrelated`, `offensive`.
    pub reasons: Vec<String>,
    /// The legal basis (to check against the original texts).
    pub legal_basis: Option<String>,
}

/// List the reasons for which reviews and other content of consumers may be hidden.
///
/// No sign-in (the tenant comes from the domain). Moderation may hide content of consumers only
/// for these reasons (legal rule `moderation.hide_reasons`; Consumer Protection Law 19/2023/QH15,
/// Decree 55/2024/ND-CP): a negative but lawful review stays visible.
#[utoipa::path(
    get,
    path = "/public/moderation-reasons",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_compliance_moderation_reasons",
    params(SurfaceQuery),
    responses(
        (status = 200, description = "The reasons.", body = Data<ModerationReasonsResponse>),
    )
)]
async fn moderation_reasons(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    Query(query): Query<SurfaceQuery>,
) -> Result<Data<ModerationReasonsResponse>, Error> {
    let surface = query.surface.unwrap_or_else(|| "reviews".to_owned());
    let (reasons, legal_basis) = service.hide_reasons(&tenant, &surface).await?;
    Ok(Data { data: ModerationReasonsResponse { surface, reasons, legal_basis } })
}
