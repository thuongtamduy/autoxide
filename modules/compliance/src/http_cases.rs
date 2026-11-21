//! Route REST của vụ việc (D-363) và bảo vệ người tiêu dùng (D-364) dưới `/api/v1/compliance`: báo vi
//! phạm sở hữu trí tuệ / hàng giả, khiếu nại, phản hồi của bên bị phản ánh, quyết định, cung cấp thông
//! tin cho cơ quan, lý do được ẩn đánh giá.

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{ClientInfo, ValidJson};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
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

mod disclosures;
mod public;

pub(crate) use public::ComplaintChannelResponse;

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
        .routes(routes!(disclosures::list_disclosures, disclosures::record_disclosure))
        .routes(routes!(public::public_report))
        .routes(routes!(public::complaint_channel, public::public_complaint))
        .routes(routes!(public::tracked_case))
        .routes(routes!(public::moderation_reasons))
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
    pub created_at: Timestamp,
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
    pub ack_due_at: Timestamp,
    pub acknowledged_at: Option<Timestamp>,
    /// The reported party may answer until this time.
    pub respond_by: Timestamp,
    /// Decide before this time (working days of the legal rule `cases.deadlines`; 24 hours for
    /// a request of an authority).
    pub decide_by: Timestamp,
    /// `true` while open and past `decide_by`.
    pub overdue: bool,
    pub decided_at: Option<Timestamp>,
    pub decided_by: Option<Uuid>,
    /// Why the anti-spam checks held it; `null` when not held.
    pub held_reason: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
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
        overdue: is_open(&row.status) && row.decide_by < Timestamp::now(),
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
    pub respond_by: Timestamp,
    pub decide_by: Timestamp,
    pub created_at: Timestamp,
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
    pub at: Timestamp,
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
    pub ack_due_at: Timestamp,
    pub acknowledged_at: Option<Timestamp>,
    pub decide_by: Timestamp,
    pub decided_at: Option<Timestamp>,
    pub created_at: Timestamp,
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
