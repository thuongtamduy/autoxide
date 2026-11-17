//! Route REST của module `tax`, dưới `/api/v1/tax`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{OrgId, TenantContext, require_platform, root_of};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::api::{
    BuyerKind, PriceMode, TaxProfile, TaxQuote, TaxQuoteRequest, TaxableLine, WithholdingQuote,
    WithholdingRequest,
};
use crate::entity::{pack, profile};
use crate::permissions::{PACKS_MANAGE, READ};
use crate::service::{ProfileInput, TaxService, describe_profile};

pub(crate) fn routes(service: Arc<TaxService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_jurisdictions))
        .routes(routes!(list_categories))
        .routes(routes!(list_rates))
        .routes(routes!(list_withholding_rates))
        .routes(routes!(list_rules))
        .routes(routes!(quote))
        .routes(routes!(withholding))
        .routes(routes!(list_profiles))
        .routes(routes!(get_profile, put_profile))
        .routes(routes!(list_packs, import_pack))
        .with_state(service)
}

/// `tax.read` ở bất kỳ nút nào (dữ liệu luật, không thuộc nút nào).
fn reader(principal: &Principal) -> Result<(), Error> {
    principal.require(&READ)
}

// ---- Dữ liệu luật -----------------------------------------------------------------------------

/// A tax jurisdiction.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = TaxJurisdictionResponse)]
pub(crate) struct JurisdictionResponse {
    #[schema(example = "VN")]
    code: String,
    name: String,
    #[schema(example = "VND")]
    default_currency: String,
    /// The data pack it comes from, and its version.
    pack_code: String,
    pack_version: i64,
}

/// A tax category.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = TaxCategoryResponse)]
pub(crate) struct CategoryResponse {
    jurisdiction: String,
    #[schema(example = "vat.standard_reducible")]
    code: String,
    name_en: String,
    name_vi: String,
    /// `vat`, `excise`, `environment`.
    tax_kind: String,
}

/// A tax rate with its dates.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = TaxRateResponse)]
pub(crate) struct RateResponse {
    id: Uuid,
    jurisdiction: String,
    category: String,
    tax_kind: String,
    /// `percent`, `per_unit` or `exempt` (not subject to the tax).
    basis: String,
    #[schema(example = "8")]
    rate_percent: String,
    per_unit: Option<Money>,
    /// Order the taxes apply in: a later stage is computed on the price plus earlier taxes.
    stage: i32,
    /// First day in force.
    effective_from: String,
    /// First day no longer in force; `null` while in force.
    effective_to: Option<String>,
    /// Legal basis (check it against the original document before production use).
    legal_basis: String,
    pack_version: i64,
}

/// A rate a platform withholds from payments to sellers.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = TaxWithholdingRateResponse)]
pub(crate) struct WithholdingRateResponse {
    id: Uuid,
    jurisdiction: String,
    /// `marketplace` or `affiliate`.
    payer_kind: String,
    payee_regime: String,
    business_line: String,
    /// `vat` or `pit`.
    tax_kind: String,
    rate_percent: String,
    threshold: Option<Money>,
    effective_from: String,
    effective_to: Option<String>,
    legal_basis: String,
}

/// A tax rule with its dates (e.g. when cash register invoices are required).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = TaxRuleResponse)]
pub(crate) struct RuleResponse {
    id: Uuid,
    jurisdiction: String,
    #[schema(example = "einvoice.cash_register_required")]
    code: String,
    #[schema(value_type = Object)]
    value: serde_json::Value,
    effective_from: String,
    effective_to: Option<String>,
    legal_basis: String,
}

/// Filters of tax data lists.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct DataQuery {
    /// Only this jurisdiction, e.g. `VN`.
    jurisdiction: Option<String>,
    /// Only this category (rates).
    category: Option<String>,
    /// Only what is in force on this day (`YYYY-MM-DD`).
    on: Option<NaiveDate>,
}

fn in_force_on(from: &str, to: Option<&String>, on: Option<NaiveDate>) -> bool {
    on.is_none_or(|on| {
        let day = on.format("%Y-%m-%d").to_string();
        from <= day.as_str() && to.is_none_or(|to| day.as_str() < to.as_str())
    })
}

/// List the tax jurisdictions.
///
/// Requires `tax.read`.
#[utoipa::path(
    get,
    path = "/jurisdictions",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "list_tax_jurisdictions",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The jurisdictions, by code.", body = Data<Vec<JurisdictionResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_jurisdictions(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
) -> Result<Data<Vec<JurisdictionResponse>>, Error> {
    reader(&principal)?;
    let tables = service.tables().await?;
    let mut list: Vec<JurisdictionResponse> = tables
        .jurisdictions
        .iter()
        .map(|row| JurisdictionResponse {
            code: row.code.clone(),
            name: row.name.clone(),
            default_currency: row.default_currency.clone(),
            pack_code: row.pack_code.clone(),
            pack_version: row.pack_version,
        })
        .collect();
    list.sort_by(|a, b| a.code.cmp(&b.code));
    Ok(Data { data: list })
}

/// List the tax categories.
///
/// Requires `tax.read`.
#[utoipa::path(
    get,
    path = "/categories",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "list_tax_categories",
    security(("bearer" = [])),
    params(DataQuery),
    responses(
        (status = 200, description = "The categories, by jurisdiction and code.", body = Data<Vec<CategoryResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_categories(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
    Query(filter): Query<DataQuery>,
) -> Result<Data<Vec<CategoryResponse>>, Error> {
    reader(&principal)?;
    let tables = service.tables().await?;
    let mut list: Vec<CategoryResponse> = tables
        .categories
        .iter()
        .filter(|row| filter.jurisdiction.as_ref().is_none_or(|code| &row.jurisdiction == code))
        .map(|row| CategoryResponse {
            jurisdiction: row.jurisdiction.clone(),
            code: row.code.clone(),
            name_en: row.name_en.clone(),
            name_vi: row.name_vi.clone(),
            tax_kind: row.tax_kind.clone(),
        })
        .collect();
    list.sort_by(|a, b| (&a.jurisdiction, &a.code).cmp(&(&b.jurisdiction, &b.code)));
    Ok(Data { data: list })
}

/// List the tax rates with their dates.
///
/// Requires `tax.read`. With `on`, only the rates in force that day.
#[utoipa::path(
    get,
    path = "/rates",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "list_tax_rates",
    security(("bearer" = [])),
    params(DataQuery),
    responses(
        (status = 200, description = "The rates, by category, stage and start date.", body = Data<Vec<RateResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_rates(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
    Query(filter): Query<DataQuery>,
) -> Result<Data<Vec<RateResponse>>, Error> {
    reader(&principal)?;
    let tables = service.tables().await?;
    let mut list: Vec<RateResponse> = tables
        .rates
        .iter()
        .filter(|row| filter.jurisdiction.as_ref().is_none_or(|code| &row.jurisdiction == code))
        .filter(|row| filter.category.as_ref().is_none_or(|code| &row.category_code == code))
        .filter(|row| in_force_on(&row.effective_from, row.effective_to.as_ref(), filter.on))
        .map(|row| RateResponse {
            id: row.id,
            jurisdiction: row.jurisdiction.clone(),
            category: row.category_code.clone(),
            tax_kind: row.tax_kind.clone(),
            basis: row.basis.clone(),
            rate_percent: row.rate_percent.clone(),
            per_unit: match (row.per_unit_minor, &row.per_unit_currency) {
                (Some(minor), Some(currency)) => Money::from_columns(minor, currency).ok(),
                _ => None,
            },
            stage: row.stage,
            effective_from: row.effective_from.clone(),
            effective_to: row.effective_to.clone(),
            legal_basis: row.legal_basis.clone(),
            pack_version: row.pack_version,
        })
        .collect();
    list.sort_by(|a, b| {
        (&a.jurisdiction, &a.category, a.stage, &a.effective_from).cmp(&(
            &b.jurisdiction,
            &b.category,
            b.stage,
            &b.effective_from,
        ))
    });
    Ok(Data { data: list })
}

/// List the withholding rates for platforms.
///
/// Requires `tax.read`. With `on`, only those in force that day.
#[utoipa::path(
    get,
    path = "/withholding-rates",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "list_tax_withholding_rates",
    security(("bearer" = [])),
    params(DataQuery),
    responses(
        (status = 200, description = "The withholding rates.", body = Data<Vec<WithholdingRateResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_withholding_rates(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
    Query(filter): Query<DataQuery>,
) -> Result<Data<Vec<WithholdingRateResponse>>, Error> {
    reader(&principal)?;
    let tables = service.tables().await?;
    let mut list: Vec<WithholdingRateResponse> = tables
        .withholding
        .iter()
        .filter(|row| filter.jurisdiction.as_ref().is_none_or(|code| &row.jurisdiction == code))
        .filter(|row| in_force_on(&row.effective_from, row.effective_to.as_ref(), filter.on))
        .map(|row| WithholdingRateResponse {
            id: row.id,
            jurisdiction: row.jurisdiction.clone(),
            payer_kind: row.payer_kind.clone(),
            payee_regime: row.payee_regime.clone(),
            business_line: row.business_line.clone(),
            tax_kind: row.tax_kind.clone(),
            rate_percent: row.rate_percent.clone(),
            threshold: match (row.threshold_minor, &row.threshold_currency) {
                (Some(minor), Some(currency)) => Money::from_columns(minor, currency).ok(),
                _ => None,
            },
            effective_from: row.effective_from.clone(),
            effective_to: row.effective_to.clone(),
            legal_basis: row.legal_basis.clone(),
        })
        .collect();
    list.sort_by(|a, b| {
        (&a.payer_kind, &a.payee_regime, &a.business_line, &a.tax_kind, &a.effective_from).cmp(&(
            &b.payer_kind,
            &b.payee_regime,
            &b.business_line,
            &b.tax_kind,
            &b.effective_from,
        ))
    });
    Ok(Data { data: list })
}

/// List the tax rules with their dates.
///
/// Requires `tax.read`. With `on`, only those in force that day.
#[utoipa::path(
    get,
    path = "/rules",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "list_tax_rules",
    security(("bearer" = [])),
    params(DataQuery),
    responses(
        (status = 200, description = "The rules.", body = Data<Vec<RuleResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_rules(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
    Query(filter): Query<DataQuery>,
) -> Result<Data<Vec<RuleResponse>>, Error> {
    reader(&principal)?;
    let tables = service.tables().await?;
    let mut list: Vec<RuleResponse> = tables
        .rules
        .iter()
        .filter(|row| filter.jurisdiction.as_ref().is_none_or(|code| &row.jurisdiction == code))
        .filter(|row| in_force_on(&row.effective_from, row.effective_to.as_ref(), filter.on))
        .map(|row| RuleResponse {
            id: row.id,
            jurisdiction: row.jurisdiction.clone(),
            code: row.code.clone(),
            value: serde_json::from_str(&row.value).unwrap_or_default(),
            effective_from: row.effective_from.clone(),
            effective_to: row.effective_to.clone(),
            legal_basis: row.legal_basis.clone(),
        })
        .collect();
    list.sort_by(|a, b| (&a.code, &a.effective_from).cmp(&(&b.code, &b.effective_from)));
    Ok(Data { data: list })
}

// ---- Tính thuế ---------------------------------------------------------------------------------

/// A document to tax.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TaxQuoteRequest)]
pub(crate) struct QuoteRequest {
    #[schema(example = "VN")]
    jurisdiction: String,
    /// Day the tax applies (`YYYY-MM-DD`, in the seller's time zone).
    tax_point: NaiveDate,
    /// The selling unit; default the tenant's root. Its tax profile (or the nearest above) decides
    /// the regime; without one the seller declares VAT by credit.
    seller_org_id: Option<Uuid>,
    /// `consumer` (default) or `business`.
    buyer_kind: Option<BuyerKind>,
    /// `exclusive`: amounts are before taxes; `inclusive`: they include them.
    price_mode: PriceMode,
    #[schema(example = "VND")]
    currency: String,
    /// 1 to 1000 lines.
    lines: Vec<TaxableLine>,
}

/// Compute the taxes of a document.
///
/// Requires `tax.read` on the selling unit. Only computes. Taxes are rounded once per rate on the
/// whole document, then shared between the lines, so the lines always add up to the totals.
#[utoipa::path(
    post,
    path = "/quote",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "quote_taxes",
    security(("bearer" = [])),
    request_body = QuoteRequest,
    responses(
        (status = 200, description = "The taxes per line and per rate.", body = Data<TaxQuote>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read` (`permission_denied`) or the unit is outside your scope (`org_out_of_scope`).", body = Problem),
        (status = 422, description = "Unknown jurisdiction (`tax.jurisdiction_unknown`) or category (`tax.category_unknown`), no rate on that day (`tax.rate_missing`), or a value is invalid (see `errors`).", body = Problem),
    )
)]
async fn quote(
    State(service): State<Arc<TaxService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<QuoteRequest>,
) -> Result<Data<TaxQuote>, Error> {
    let seller = input.seller_org_id.map_or_else(|| root_of(tenant.id()), OrgId::from_uuid);
    service.require_seller(&tenant, &principal, seller).await?;
    let currency = Currency::parse(&input.currency)?;
    let mut request = TaxQuoteRequest::new(
        input.jurisdiction,
        input.tax_point,
        seller,
        input.price_mode,
        currency,
        input.lines,
    );
    if input.buyer_kind == Some(BuyerKind::Business) {
        request = request.for_business();
    }
    Ok(Data { data: service.quote(&tenant, request).await? })
}

/// Tax a platform withholds.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TaxWithholdingInput)]
pub(crate) struct WithholdingInput {
    #[schema(example = "VN")]
    jurisdiction: String,
    /// Day of the payment.
    on: NaiveDate,
    /// `marketplace` or `affiliate`.
    #[schema(example = "marketplace")]
    payer_kind: String,
    /// The payee's regime: `household`… (a business declares itself: nothing is withheld).
    #[schema(example = "household")]
    payee_regime: String,
    /// `goods`, `services`, `production_transport` or `other`.
    #[schema(example = "goods")]
    business_line: String,
    amount: Money,
}

/// Compute the taxes a platform withholds from a payment.
///
/// Requires `tax.read`. Only computes.
#[utoipa::path(
    post,
    path = "/withholding",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "quote_withholding",
    security(("bearer" = [])),
    request_body = WithholdingInput,
    responses(
        (status = 200, description = "The taxes to withhold (empty when none applies).", body = Data<WithholdingQuote>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Unknown jurisdiction (`tax.jurisdiction_unknown`) or a value is invalid (see `errors`).", body = Problem),
    )
)]
async fn withholding(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
    ValidJson(input): ValidJson<WithholdingInput>,
) -> Result<Data<WithholdingQuote>, Error> {
    reader(&principal)?;
    let request = WithholdingRequest::new(
        input.jurisdiction,
        input.on,
        input.payer_kind,
        input.payee_regime,
        input.business_line,
        input.amount,
    );
    Ok(Data { data: service.withholding(request).await? })
}

// ---- Hồ sơ thuế ------------------------------------------------------------------------------

/// The tax profile that applies to a unit.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = TaxProfileResponse)]
pub(crate) struct ProfileResponse {
    #[serde(flatten)]
    profile: TaxProfile,
    /// `true` when the profile is the unit's own; `false` when it comes from a unit above.
    own: bool,
    updated_by: Option<Uuid>,
    updated_at: DateTime<Utc>,
}

fn respond(row: &profile::Model, asked: Option<OrgId>) -> ProfileResponse {
    ProfileResponse {
        profile: describe_profile(row),
        own: asked.is_none_or(|org| org.as_uuid() == row.org_id),
        updated_by: row.updated_by,
        updated_at: row.updated_at,
    }
}

/// The whole tax profile of a unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TaxProfileRequest)]
pub(crate) struct ProfileRequest {
    #[schema(example = "VN")]
    jurisdiction: String,
    /// 10 or 13 digits (`0101234567-001` for a branch), or a 12-digit personal id used as tax
    /// code.
    tax_code: Option<String>,
    /// `vat_credit`, `vat_direct`, `household` or `exempt`.
    #[schema(example = "vat_credit")]
    regime: String,
    /// `goods`, `services`, `production_transport`, `other`.
    #[serde(default)]
    business_lines: Vec<String>,
    /// Estimated revenue a year (thresholds such as cash register invoices).
    annual_revenue: Option<Money>,
    /// Up to 200 characters.
    legal_name: Option<String>,
    /// Address; names are filled in from the codes.
    address: Option<autoxide_address::api::AddressInput>,
}

/// List the tax profiles in your scope.
///
/// Requires `tax.read`. Only profiles set on a unit (not inherited ones).
#[utoipa::path(
    get,
    path = "/profiles",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "list_tax_profiles",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of profiles.", body = Page<ProfileResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_profiles(
    State(service): State<Arc<TaxService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ProfileResponse>, Error> {
    let page = service.list_profiles(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(|row| respond(&row, None)))
}

/// Get the tax profile that applies to a unit.
///
/// Requires `tax.read` on the unit. A unit without its own profile gets the nearest one above it
/// (`own: false`).
#[utoipa::path(
    get,
    path = "/profiles/{org_id}",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "get_tax_profile",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Id of the organization unit.")),
    responses(
        (status = 200, description = "The profile that applies.", body = Data<ProfileResponse>, headers(("ETag" = String, description = "Version of the profile: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No profile on the unit or above it, or the unit is outside your scope; code `tax.profile_not_found`.", body = Problem),
    )
)]
async fn get_profile(
    State(service): State<Arc<TaxService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Tagged<ProfileResponse>, Error> {
    let org = OrgId::from_uuid(org_id);
    let row = service.read_profile(&tenant, &principal, org).await?;
    Ok(Tagged { version: row.updated_at, data: respond(&row, Some(org)) })
}

/// Set the tax profile of a unit.
///
/// Requires `tax.profiles_manage` on the unit. The body replaces the unit's own profile (units
/// below without their own profile inherit it).
#[utoipa::path(
    put,
    path = "/profiles/{org_id}",
    extensions(("x-permissions" = json!(["tax.profiles_manage"]))),
    operation_id = "set_tax_profile",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Id of the organization unit."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the profile since; otherwise `412`.")),
    request_body = ProfileRequest,
    responses(
        (status = 200, description = "The profile after the change.", body = Data<ProfileResponse>, headers(("ETag" = String, description = "Version of the profile."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.profiles_manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or frozen (`org_frozen`).", body = Problem),
        (status = 412, description = "`If-Match` does not match; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn put_profile(
    State(service): State<Arc<TaxService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ProfileRequest>,
) -> Result<Tagged<ProfileResponse>, Error> {
    let org = OrgId::from_uuid(org_id);
    let input = ProfileInput {
        jurisdiction: input.jurisdiction,
        tax_code: input.tax_code,
        regime: input.regime,
        business_lines: input.business_lines,
        annual_revenue: input.annual_revenue,
        legal_name: input.legal_name,
        address: input.address,
    };
    let row = service.set_profile(&tenant, &principal, org, input, if_match.expected()?).await?;
    Ok(Tagged { version: row.updated_at, data: respond(&row, Some(org)) })
}

// ---- Gói dữ liệu ------------------------------------------------------------------------------

/// An imported tax data pack.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = TaxPackResponse)]
pub(crate) struct PackResponse {
    id: Uuid,
    #[schema(example = "vn")]
    code: String,
    version: i64,
    /// Day its sources were last checked.
    checked_on: String,
    source_note: String,
    content_sha256: String,
    imported_by: Option<Uuid>,
    imported_at: DateTime<Utc>,
}

impl From<pack::Model> for PackResponse {
    fn from(row: pack::Model) -> Self {
        PackResponse {
            id: row.id,
            code: row.code,
            version: row.version,
            checked_on: row.checked_on,
            source_note: row.source_note,
            content_sha256: row.content_sha256,
            imported_by: row.imported_by,
            imported_at: row.imported_at,
        }
    }
}

/// A tax data pack to import.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = TaxPackRequest)]
pub(crate) struct PackRequest {
    /// The pack, in the format of `modules/tax/packs/vn.json`: `code`, `version`, `checked_on`,
    /// `source_note`, `jurisdictions`, `categories`, `rates`, `withholding_rates`, `rules`.
    #[schema(value_type = Object)]
    pack: serde_json::Value,
}

/// List the imported tax data packs.
///
/// Requires `tax.read`.
#[utoipa::path(
    get,
    path = "/packs",
    extensions(("x-permissions" = json!(["tax.read"]))),
    operation_id = "list_tax_packs",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The packs, newest version first per code.", body = Data<Vec<PackResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `tax.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_packs(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
) -> Result<Data<Vec<PackResponse>>, Error> {
    reader(&principal)?;
    Ok(Data { data: service.packs().await?.into_iter().map(PackResponse::from).collect() })
}

/// Import a tax data pack.
///
/// Platform staff with `tax.packs_manage`. Replaces the data of the jurisdictions the pack lists,
/// for every tenant. Only a higher version than the one imported is accepted; the same pack again
/// changes nothing.
#[utoipa::path(
    post,
    path = "/packs",
    extensions(("x-permissions" = json!(["tax.packs_manage"]))),
    operation_id = "import_tax_pack",
    security(("bearer" = [])),
    request_body = PackRequest,
    responses(
        (status = 200, description = "The pack in use for this code after the import.", body = Data<PackResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Not platform staff (`tenant_platform_only`) or missing `tax.packs_manage` (`permission_denied`).", body = Problem),
        (status = 409, description = "A version as high or higher is imported; code `tax.pack_outdated`.", body = Problem),
        (status = 422, description = "The pack is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn import_pack(
    State(service): State<Arc<TaxService>>,
    principal: Principal,
    ValidJson(input): ValidJson<PackRequest>,
) -> Result<Data<PackResponse>, Error> {
    require_platform(&principal)?;
    principal.require(&PACKS_MANAGE)?;
    let text = input.pack.to_string();
    service.import_pack(Actor::from(&principal), &text).await?;
    let code = input.pack["code"].as_str().unwrap_or_default().to_owned();
    let row = service
        .latest_pack(&code)
        .await?
        .ok_or_else(|| Error::internal("an imported tax pack vanished"))?;
    Ok(Data { data: row.into() })
}
