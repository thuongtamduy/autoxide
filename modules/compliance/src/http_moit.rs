//! Route REST của hồ sơ pháp lý, website, danh sách kiểm, bộ hồ sơ Bộ Công Thương và hai endpoint
//! công khai cho giao diện (logo, thông tin chủ sở hữu) — dưới `/api/v1/compliance`.

use std::sync::Arc;

use autoxide_address::api::{Address, AddressInput};
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext, TenantHint};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::checklist::{Checklist, Dossier};
use crate::entity::{legal_profile, website, website_page};
use crate::legal::{ProfileFields, address_of, profile_not_found};
use crate::service::ComplianceService;
use crate::websites::{NewWebsite, StatusChange, WebsiteChange};

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_profiles, create_profile))
        .routes(routes!(get_profile, update_profile))
        .routes(routes!(profile_variables))
        .routes(routes!(list_websites, create_website))
        .routes(routes!(get_website, update_website))
        .routes(routes!(website_checklist))
        .routes(routes!(website_dossier))
        .routes(routes!(link_page, unlink_page))
        .routes(routes!(set_status))
        .routes(routes!(public_badge))
        .routes(routes!(public_owner))
        .with_state(service)
}

/// Một trường có mặt trong thân (kể cả `null`) → `Some(…)`; vắng → `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

// ---- Hồ sơ pháp lý -------------------------------------------------------------------------------

/// The legal owner of a website (a tenant, or a seller unit): the information Vietnamese law
/// requires a commerce website to show, and the declaration to the Ministry of Industry and Trade.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceLegalProfile)]
pub(crate) struct LegalProfileResponse {
    pub id: Uuid,
    /// Organization unit it belongs to; the root unit (the tenant id) is the whole tenant.
    pub org_id: Uuid,
    /// `individual`, `household_business` or `organization`.
    pub entity_type: String,
    pub legal_name: String,
    pub trade_name: Option<String>,
    /// 10 or 12 digits, or 10 digits, `-` and 3 digits.
    pub tax_code: Option<String>,
    /// Business registration certificate number (an individual: identity document number).
    pub registration_number: Option<String>,
    /// `YYYY-MM-DD`.
    pub registration_issued_on: Option<String>,
    pub registration_issued_by: Option<String>,
    pub address: Option<Address>,
    pub representative_name: Option<String>,
    pub representative_title: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    /// Identity verification (KYC) profile of the owner.
    pub kyc_profile_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<legal_profile::Model> for LegalProfileResponse {
    fn from(row: legal_profile::Model) -> Self {
        LegalProfileResponse {
            address: address_of(&row),
            id: row.id,
            org_id: row.org_id,
            entity_type: row.entity_type,
            legal_name: row.legal_name,
            trade_name: row.trade_name,
            tax_code: row.tax_code,
            registration_number: row.registration_number,
            registration_issued_on: row.registration_issued_on,
            registration_issued_by: row.registration_issued_by,
            representative_name: row.representative_name,
            representative_title: row.representative_title,
            phone: row.phone,
            email: row.email,
            kyc_profile_id: row.kyc_profile_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A legal profile to create.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceLegalProfileRequest)]
pub(crate) struct ProfileRequest {
    /// Organization unit (e.g. a seller); left out: the whole tenant. One profile per unit.
    org_id: Option<Uuid>,
    /// `individual`, `household_business` or `organization`.
    #[schema(example = "organization")]
    entity_type: String,
    /// Registered name, 1 to 200 characters.
    #[schema(example = "Cong ty TNHH Vi Du")]
    legal_name: String,
    trade_name: Option<String>,
    #[schema(example = "0101234567")]
    tax_code: Option<String>,
    registration_number: Option<String>,
    /// `YYYY-MM-DD`.
    registration_issued_on: Option<String>,
    registration_issued_by: Option<String>,
    /// Checked against the administrative units (module `address`).
    address: Option<AddressInput>,
    representative_name: Option<String>,
    representative_title: Option<String>,
    phone: Option<String>,
    email: Option<String>,
    kyc_profile_id: Option<Uuid>,
}

/// Changes to a legal profile: a field left out stays; `""` clears a text, `null` clears
/// `address` or `kyc_profile_id`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceLegalProfileChange)]
pub(crate) struct ProfileChange {
    entity_type: Option<String>,
    legal_name: Option<String>,
    trade_name: Option<String>,
    tax_code: Option<String>,
    registration_number: Option<String>,
    registration_issued_on: Option<String>,
    registration_issued_by: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<AddressInput>, nullable)]
    address: Option<Option<AddressInput>>,
    representative_name: Option<String>,
    representative_title: Option<String>,
    phone: Option<String>,
    email: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    kyc_profile_id: Option<Option<Uuid>>,
}

/// List the legal profiles you can see.
///
/// Requires `compliance.legal.read` (limited to the units in your scope).
#[utoipa::path(
    get,
    path = "/legal-profiles",
    extensions(("x-permissions" = json!(["compliance.legal.read"]))),
    operation_id = "list_compliance_legal_profiles",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of legal profiles.", body = Page<LegalProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_profiles(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<LegalProfileResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_READ)?;
    let page =
        service.list_profiles_legal(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(LegalProfileResponse::from))
}

/// Create the legal profile of the tenant or of a unit.
///
/// Requires `compliance.legal.manage` on that unit. Every problem of the body is reported at once.
#[utoipa::path(
    post,
    path = "/legal-profiles",
    extensions(("x-permissions" = json!(["compliance.legal.manage"]))),
    operation_id = "create_compliance_legal_profile",
    security(("bearer" = [])),
    request_body = ProfileRequest,
    responses(
        (status = 201, description = "The profile.", body = Data<LegalProfileResponse>, headers(("Location" = String, description = "Path of the profile."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.manage` (code `permission_denied`), or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No such organization unit; code `site_not_found`.", body = Problem),
        (status = 409, description = "The unit already has a profile; code `compliance.profile_exists`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_profile(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ProfileRequest>,
) -> Result<Created<LegalProfileResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_MANAGE)?;
    let fields = ProfileFields {
        entity_type: Some(input.entity_type),
        legal_name: Some(input.legal_name),
        trade_name: input.trade_name,
        tax_code: input.tax_code,
        registration_number: input.registration_number,
        registration_issued_on: input.registration_issued_on,
        registration_issued_by: input.registration_issued_by,
        address: input.address.map(Some),
        representative_name: input.representative_name,
        representative_title: input.representative_title,
        phone: input.phone,
        email: input.email,
        kyc_profile_id: input.kyc_profile_id.map(Some),
    };
    let row = service.create_profile(&tenant, &principal, input.org_id, fields).await?;
    Ok(Created::new(format!("/compliance/legal-profiles/{}", row.id), row.into()))
}

/// Get a legal profile.
///
/// Requires `compliance.legal.read` on its unit. Sends an `ETag` to use in `If-Match`.
#[utoipa::path(
    get,
    path = "/legal-profiles/{id}",
    extensions(("x-permissions" = json!(["compliance.legal.read"]))),
    operation_id = "get_compliance_legal_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    responses(
        (status = 200, description = "The profile.", body = Data<LegalProfileResponse>, headers(("ETag" = String, description = "Version, for `If-Match`."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such profile in your scope; code `compliance.profile_not_found`.", body = Problem),
    )
)]
async fn get_profile(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<LegalProfileResponse>, Error> {
    let row = service.get_profile(&tenant, &principal, id).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// Change a legal profile.
///
/// Requires `compliance.legal.manage` on its unit. Send `If-Match` with the `ETag` you read to
/// avoid overwriting someone else's change.
#[utoipa::path(
    patch,
    path = "/legal-profiles/{id}",
    extensions(("x-permissions" = json!(["compliance.legal.manage"]))),
    operation_id = "update_compliance_legal_profile",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile."), ("If-Match" = Option<String>, Header, description = "`ETag` of the version you read.")),
    request_body = ProfileChange,
    responses(
        (status = 200, description = "The profile after the change.", body = Data<LegalProfileResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such profile in your scope; code `compliance.profile_not_found`.", body = Problem),
        (status = 412, description = "Changed since you read it; code `precondition_failed`. Read it again.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn update_profile(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    if_match: IfMatch,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ProfileChange>,
) -> Result<Tagged<LegalProfileResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_MANAGE)?;
    let fields = ProfileFields {
        entity_type: input.entity_type,
        legal_name: input.legal_name,
        trade_name: input.trade_name,
        tax_code: input.tax_code,
        registration_number: input.registration_number,
        registration_issued_on: input.registration_issued_on,
        registration_issued_by: input.registration_issued_by,
        address: input.address,
        representative_name: input.representative_name,
        representative_title: input.representative_title,
        phone: input.phone,
        email: input.email,
        kyc_profile_id: input.kyc_profile_id,
    };
    let row = service.update_profile(&tenant, &principal, id, if_match.expected()?, fields).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// Get the template variables of a legal profile.
///
/// Requires `compliance.legal.read`. The values (`owner.*`, `contact.*`) fill the mandatory page
/// templates of `content` (`POST /api/v1/content/templates/{key}/drafts`).
#[utoipa::path(
    get,
    path = "/legal-profiles/{id}/template-variables",
    extensions(("x-permissions" = json!(["compliance.legal.read"]))),
    operation_id = "get_compliance_legal_profile_variables",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the profile.")),
    responses(
        (status = 200, description = "The variables.", body = Data<serde_json::Value>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such profile in your scope; code `compliance.profile_not_found`.", body = Problem),
    )
)]
async fn profile_variables(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<serde_json::Value>, Error> {
    let row = service.get_profile(&tenant, &principal, id).await?;
    Ok(Data { data: ComplianceService::template_variables_of(&row) })
}

// ---- Website ----------------------------------------------------------------------------------

/// A mandatory page linked to a website.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceWebsitePage)]
pub(crate) struct PageResponse {
    /// e.g. `return_policy`.
    pub page_kind: String,
    /// The linked object, e.g. `content.entry:<id>`.
    pub resource: String,
    pub linked_at: DateTime<Utc>,
}

impl From<website_page::Model> for PageResponse {
    fn from(row: website_page::Model) -> Self {
        PageResponse { page_kind: row.page_kind, resource: row.resource, linked_at: row.linked_at }
    }
}

/// A website (or mobile app) and its procedure with the Ministry of Industry and Trade.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceWebsite)]
pub(crate) struct WebsiteResponse {
    pub id: Uuid,
    /// Organization unit of the website; the root unit (the tenant id) is the whole tenant.
    pub org_id: Uuid,
    #[schema(example = "shop.example")]
    pub domain: String,
    /// `website` or `mobile_app`.
    pub channel: String,
    /// `sales` (notify the ministry, blue logo) or `platform` (marketplace or other
    /// intermediary: register with the ministry, red logo).
    pub kind: String,
    /// `not_submitted`, `submitted`, `needs_supplement`, `notified`, `registered`, `rejected` or
    /// `revoked`.
    pub status: String,
    /// Number of the file at the ministry.
    pub reference: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub decided_at: Option<DateTime<Utc>>,
    /// Logo link and image, once `notified` / `registered`.
    pub logo_link_url: Option<String>,
    pub logo_image_url: Option<String>,
    /// Checklist version when it was submitted.
    pub checklist_version: Option<i32>,
    /// Linked mandatory pages (only on `GET /websites/{id}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pages: Option<Vec<PageResponse>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub(crate) fn describe_website(
    row: website::Model,
    pages: Option<Vec<website_page::Model>>,
) -> WebsiteResponse {
    WebsiteResponse {
        id: row.id,
        org_id: row.org_id,
        domain: row.domain,
        channel: row.channel,
        kind: row.website_kind,
        status: row.moit_status,
        reference: row.moit_reference,
        submitted_at: row.submitted_at,
        decided_at: row.decided_at,
        logo_link_url: row.logo_link_url,
        logo_image_url: row.logo_image_url,
        checklist_version: row.checklist_version,
        pages: pages.map(|rows| rows.into_iter().map(PageResponse::from).collect()),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// A website to add.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceWebsiteRequest)]
pub(crate) struct WebsiteRequest {
    /// Organization unit (e.g. a seller's own site); left out: the whole tenant.
    org_id: Option<Uuid>,
    /// Domain name (international names in punycode).
    #[schema(example = "shop.example")]
    domain: String,
    /// `sales` or `platform`.
    #[schema(example = "sales")]
    kind: String,
    /// `website` (default) or `mobile_app`.
    channel: Option<String>,
}

/// Changes to a website; a field left out stays. Changing `kind` starts the procedure again.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceWebsiteChange)]
pub(crate) struct WebsiteChangeRequest {
    org_id: Option<Uuid>,
    domain: Option<String>,
    kind: Option<String>,
    channel: Option<String>,
}

/// List the websites you can see.
///
/// Requires `compliance.legal.read` (limited to the units in your scope).
#[utoipa::path(
    get,
    path = "/websites",
    extensions(("x-permissions" = json!(["compliance.legal.read"]))),
    operation_id = "list_compliance_websites",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of websites.", body = Page<WebsiteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_websites(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<WebsiteResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_READ)?;
    let page = service.list_websites(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(|row| describe_website(row, None)))
}

/// Add a website for the Ministry of Industry and Trade procedure.
///
/// Requires `compliance.legal.manage` on its unit. A sales website must be notified to the
/// ministry; a marketplace or other intermediary must be registered (check with a legal
/// professional which applies).
#[utoipa::path(
    post,
    path = "/websites",
    extensions(("x-permissions" = json!(["compliance.legal.manage"]))),
    operation_id = "create_compliance_website",
    security(("bearer" = [])),
    request_body = WebsiteRequest,
    responses(
        (status = 201, description = "The website.", body = Data<WebsiteResponse>, headers(("Location" = String, description = "Path of the website."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.manage` (code `permission_denied`), or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 404, description = "No such organization unit; code `site_not_found`.", body = Problem),
        (status = 409, description = "The domain already has a website record; code `compliance.domain_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn create_website(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<WebsiteRequest>,
) -> Result<Created<WebsiteResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_MANAGE)?;
    let input = NewWebsite {
        org_id: input.org_id,
        domain: input.domain,
        kind: input.kind,
        channel: input.channel,
    };
    let row = service.create_website(&tenant, &principal, input).await?;
    Ok(Created::new(format!("/compliance/websites/{}", row.id), describe_website(row, None)))
}

/// Get a website with its linked pages.
///
/// Requires `compliance.legal.read` on its unit. Sends an `ETag` to use in `If-Match`.
#[utoipa::path(
    get,
    path = "/websites/{id}",
    extensions(("x-permissions" = json!(["compliance.legal.read"]))),
    operation_id = "get_compliance_website",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the website.")),
    responses(
        (status = 200, description = "The website.", body = Data<WebsiteResponse>, headers(("ETag" = String, description = "Version, for `If-Match`."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such website in your scope; code `compliance.website_not_found`.", body = Problem),
    )
)]
async fn get_website(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<WebsiteResponse>, Error> {
    let row = service.get_website(&tenant, &principal, id).await?;
    let pages = service.website_pages(&tenant, row.id).await?;
    Ok(Tagged { version: row.updated_at, data: describe_website(row, Some(pages)) })
}

/// Change a website.
///
/// Requires `compliance.legal.manage` on its unit. Send `If-Match` with the `ETag` you read.
#[utoipa::path(
    patch,
    path = "/websites/{id}",
    extensions(("x-permissions" = json!(["compliance.legal.manage"]))),
    operation_id = "update_compliance_website",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the website."), ("If-Match" = Option<String>, Header, description = "`ETag` of the version you read.")),
    request_body = WebsiteChangeRequest,
    responses(
        (status = 200, description = "The website after the change.", body = Data<WebsiteResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such website in your scope; code `compliance.website_not_found`.", body = Problem),
        (status = 409, description = "The new domain already has a website record; code `compliance.domain_taken`.", body = Problem),
        (status = 412, description = "Changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn update_website(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    if_match: IfMatch,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<WebsiteChangeRequest>,
) -> Result<Tagged<WebsiteResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_MANAGE)?;
    let change = WebsiteChange {
        org_id: input.org_id,
        domain: input.domain,
        kind: input.kind,
        channel: input.channel,
    };
    let row = service.update_website(&tenant, &principal, id, if_match.expected()?, change).await?;
    Ok(Tagged { version: row.updated_at, data: describe_website(row, None) })
}

/// Get what a website still lacks for the Ministry of Industry and Trade.
///
/// Requires `compliance.legal.read`. The checklist is data (rule `moit.checklist.<kind>` in
/// force); each item says `met`, `missing`, `unknown` or `manual`, with what to do.
#[utoipa::path(
    get,
    path = "/websites/{id}/checklist",
    extensions(("x-permissions" = json!(["compliance.legal.read"]))),
    operation_id = "get_compliance_website_checklist",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the website.")),
    responses(
        (status = 200, description = "The checklist.", body = Data<Checklist>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such website in your scope; code `compliance.website_not_found`.", body = Problem),
    )
)]
async fn website_checklist(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Checklist>, Error> {
    Ok(Data { data: service.checklist(&tenant, &principal, id).await? })
}

/// Export the draft dossier of a website for the Ministry of Industry and Trade.
///
/// Requires `compliance.legal.read`. Filled from the real data: declaration fields, and for a
/// marketplace a draft service project plan and operating regulations (Markdown). A person submits
/// it on online.gov.vn; have a legal professional review it first.
#[utoipa::path(
    get,
    path = "/websites/{id}/dossier",
    extensions(("x-permissions" = json!(["compliance.legal.read"]))),
    operation_id = "get_compliance_website_dossier",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the website.")),
    responses(
        (status = 200, description = "The dossier.", body = Data<Dossier>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such website in your scope; code `compliance.website_not_found`.", body = Problem),
    )
)]
async fn website_dossier(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Dossier>, Error> {
    Ok(Data { data: service.dossier(&tenant, &principal, id).await? })
}

/// A page to link.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceWebsitePageRequest)]
pub(crate) struct LinkPageRequest {
    /// A page (or post) of `content`; publish it so the checklist counts it.
    entry_id: Uuid,
}

/// Link a mandatory page to a website.
///
/// Requires `compliance.legal.manage`. `page_kind`: `owner_info`, `privacy_policy`,
/// `return_policy`, `shipping_policy`, `payment_policy`, `inspection_policy`, `complaint_policy`,
/// `terms_of_use`, `general_conditions`, `platform_rules` or `seller_agreement`. Replaces the page
/// linked before.
#[utoipa::path(
    put,
    path = "/websites/{id}/pages/{page_kind}",
    extensions(("x-permissions" = json!(["compliance.legal.manage"]))),
    operation_id = "link_compliance_website_page",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the website."), ("page_kind" = String, Path, description = "Kind of mandatory page, e.g. `return_policy`.")),
    request_body = LinkPageRequest,
    responses(
        (status = 200, description = "The link.", body = Data<PageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such website in your scope; code `compliance.website_not_found`.", body = Problem),
        (status = 422, description = "Unknown `page_kind`, or `entry_id` is not a page of this tenant; every problem in `errors`.", body = Problem),
    )
)]
async fn link_page(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, page_kind)): Path<(Uuid, String)>,
    ValidJson(input): ValidJson<LinkPageRequest>,
) -> Result<Data<PageResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_MANAGE)?;
    let row = service.link_page(&tenant, &principal, id, &page_kind, input.entry_id).await?;
    Ok(Data { data: row.into() })
}

/// Unlink a mandatory page from a website.
///
/// Requires `compliance.legal.manage`. Unlinking a page not linked changes nothing.
#[utoipa::path(
    delete,
    path = "/websites/{id}/pages/{page_kind}",
    extensions(("x-permissions" = json!(["compliance.legal.manage"]))),
    operation_id = "unlink_compliance_website_page",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the website."), ("page_kind" = String, Path, description = "Kind of mandatory page.")),
    responses(
        (status = 204, description = "Unlinked."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such website in your scope; code `compliance.website_not_found`.", body = Problem),
    )
)]
async fn unlink_page(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, page_kind)): Path<(Uuid, String)>,
) -> Result<StatusCode, Error> {
    principal.require(&crate::permissions::LEGAL_MANAGE)?;
    service.unlink_page(&tenant, &principal, id, &page_kind).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A status to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ComplianceWebsiteStatusRequest)]
pub(crate) struct StatusRequest {
    /// `not_submitted`, `submitted`, `needs_supplement`, `notified` (sales website),
    /// `registered` (marketplace), `rejected` or `revoked`.
    #[schema(example = "submitted")]
    status: String,
    /// Number of the file at the ministry.
    reference: Option<String>,
    /// For `notified` / `registered`: the logo code the ministry gave, pasted unchanged. Only a
    /// link to online.gov.vn around an image from online.gov.vn is accepted; it is rebuilt, never
    /// stored as sent.
    snippet: Option<String>,
    /// Instead of `snippet`: the logo's link…
    link_url: Option<String>,
    /// …and image.
    image_url: Option<String>,
}

/// Record the status of a website with the Ministry of Industry and Trade.
///
/// Requires `compliance.legal.manage`. A person submits on online.gov.vn (no public API), then
/// records here what happened. `notified` / `registered` need the logo; another status removes it.
#[utoipa::path(
    post,
    path = "/websites/{id}/status",
    extensions(("x-permissions" = json!(["compliance.legal.manage"]))),
    operation_id = "set_compliance_website_status",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the website."), ("If-Match" = Option<String>, Header, description = "`ETag` of the version you read.")),
    request_body = StatusRequest,
    responses(
        (status = 200, description = "The website after the change.", body = Data<WebsiteResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `compliance.legal.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such website in your scope; code `compliance.website_not_found`.", body = Problem),
        (status = 412, description = "Changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The logo code is not the ministry's (`compliance.logo_snippet_invalid`), or a value is invalid (every problem in `errors`).", body = Problem),
    )
)]
async fn set_status(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
    if_match: IfMatch,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<StatusRequest>,
) -> Result<Tagged<WebsiteResponse>, Error> {
    principal.require(&crate::permissions::LEGAL_MANAGE)?;
    let change = StatusChange {
        status: input.status,
        reference: input.reference,
        link_url: input.link_url,
        image_url: input.image_url,
        snippet: input.snippet,
    };
    let row =
        service.set_website_status(&tenant, &principal, id, if_match.expected()?, change).await?;
    Ok(Tagged { version: row.updated_at, data: describe_website(row, None) })
}

// ---- Công khai --------------------------------------------------------------------------------

/// Which website.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BadgeQuery {
    /// Domain of the website, when the interface does not call on that domain.
    domain: Option<String>,
    /// Language of the logo text: `vi` (default) or `en`.
    locale: Option<String>,
}

/// The ministry's logo for a website footer.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceBadge)]
pub(crate) struct BadgeResponse {
    pub website_id: Uuid,
    pub domain: String,
    /// `sales` or `platform`.
    pub kind: String,
    /// `notified` (blue logo) or `registered` (red logo).
    pub status: String,
    /// `blue` or `red`.
    pub color: String,
    pub link_url: String,
    pub image_url: String,
    pub alt: String,
    /// Ready-to-insert HTML, rebuilt from the checked addresses.
    pub snippet: String,
}

/// Get the Ministry of Industry and Trade logo of a website.
///
/// No sign-in (the tenant and website come from the domain, `?domain=` or `?site=`). Only a
/// website `notified` / `registered` has a logo; otherwise 404.
#[utoipa::path(
    get,
    path = "/public/badge",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_compliance_badge",
    params(BadgeQuery, SiteQuery),
    responses(
        (status = 200, description = "The logo.", body = Data<BadgeResponse>, headers(("ETag" = String, description = "Version; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The website has no logo (yet); code `compliance.website_not_found`.", body = Problem),
    )
)]
async fn public_badge(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    site: Site,
    headers: HeaderMap,
    Query(query): Query<BadgeQuery>,
) -> Result<Tagged<BadgeResponse>, Error> {
    let mut domains: Vec<String> = query.domain.into_iter().collect();
    if let Some(host) = TenantHint::from_headers(&headers).host() {
        domains.push(host.to_owned());
    }
    let row = service.public_website(&tenant, &domains, site.org()).await?;
    let rule = service.logo_rule(&tenant, &row.website_kind).await?;
    let locale = query.locale.as_deref().unwrap_or("vi");
    let logo = rule.logo(&row, locale).ok_or_else(crate::websites::website_not_found)?;
    Ok(Tagged {
        version: row.updated_at,
        data: BadgeResponse {
            website_id: row.id,
            domain: row.domain,
            kind: row.website_kind,
            status: row.moit_status,
            color: logo.color,
            link_url: logo.link_url,
            image_url: logo.image_url,
            alt: logo.alt,
            snippet: logo.snippet,
        },
    })
}

/// The website owner's information for a footer (what the law requires a commerce website to show).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CompliancePublicOwner)]
pub(crate) struct OwnerResponse {
    /// `individual`, `household_business` or `organization`.
    pub entity_type: String,
    pub legal_name: String,
    pub trade_name: Option<String>,
    pub tax_code: Option<String>,
    pub registration_number: Option<String>,
    pub registration_issued_on: Option<String>,
    pub registration_issued_by: Option<String>,
    pub address: Option<Address>,
    pub representative_name: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
}

/// Get the owner of the website being viewed.
///
/// No sign-in (the tenant and website come from the domain or `?site=`): the legal profile of the
/// website's unit, or of the nearest unit above with one.
#[utoipa::path(
    get,
    path = "/public/owner",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_compliance_owner",
    params(SiteQuery),
    responses(
        (status = 200, description = "The owner.", body = Data<OwnerResponse>, headers(("ETag" = String, description = "Version; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "No legal profile yet; code `compliance.profile_not_found`.", body = Problem),
    )
)]
async fn public_owner(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    site: Site,
) -> Result<Tagged<OwnerResponse>, Error> {
    let row = service.profile_for_org(&tenant, site.org()).await?.ok_or_else(profile_not_found)?;
    Ok(Tagged {
        version: row.updated_at,
        data: OwnerResponse {
            address: address_of(&row),
            entity_type: row.entity_type,
            legal_name: row.legal_name,
            trade_name: row.trade_name,
            tax_code: row.tax_code,
            registration_number: row.registration_number,
            registration_issued_on: row.registration_issued_on,
            registration_issued_by: row.registration_issued_by,
            representative_name: row.representative_name,
            phone: row.phone,
            email: row.email,
        },
    })
}
