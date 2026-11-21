//! Route cung cấp thông tin cho cơ quan (D-364).

use super::*;
use autoxide_kernel::time::Timestamp;

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
    pub requested_at: Timestamp,
    /// `org:<unit id>` or `user:<user id>`.
    pub subject: String,
    /// Names of the fields given (the values are not kept).
    pub fields: Vec<String>,
    pub disclosed_by: Uuid,
    pub disclosed_at: Timestamp,
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
    pub requested_at: Option<Timestamp>,
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
pub(super) async fn list_disclosures(
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
pub(super) async fn record_disclosure(
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
