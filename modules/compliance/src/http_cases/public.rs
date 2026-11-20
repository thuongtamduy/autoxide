//! Route công khai: báo vi phạm, khiếu nại, tra vụ, lý do được ẩn đánh giá.

use super::*;

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
pub(super) async fn public_report(
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

pub(super) fn receipt((id, token): (Uuid, String)) -> Result<Created<ReceiptResponse>, Error> {
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
pub(super) async fn public_complaint(
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
pub(super) async fn complaint_channel(
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
pub(super) async fn tracked_case(
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
pub(super) async fn moderation_reasons(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    Query(query): Query<SurfaceQuery>,
) -> Result<Data<ModerationReasonsResponse>, Error> {
    let surface = query.surface.unwrap_or_else(|| "reviews".to_owned());
    let (reasons, legal_basis) = service.hide_reasons(&tenant, &surface).await?;
    Ok(Data { data: ModerationReasonsResponse { surface, reasons, legal_basis } })
}
