//! Endpoint REST của module `reporting`, dưới `/api/v1/reporting`. Handler chỉ: lấy dữ liệu vào →
//! kiểm permission → gọi service → trả envelope.

use autoxide_kernel::time::{Date, Timestamp};
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::entity::fact;
use crate::permissions::{REPORTING_MANAGE, REPORTING_READ};
use crate::service::{
    CurrencyTotal, Filter, GroupBy, KindInfo, Lag, MissingRate, Reporting, Summary, SummaryGroup,
    SummaryQuery,
};

pub(crate) fn routes(service: Arc<Reporting>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_kinds))
        .routes(routes!(summary))
        .routes(routes!(list_facts))
        .routes(routes!(lag))
        .routes(routes!(rebuild))
        .with_state(service)
}

/// A kind of report figure a module declares.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct KindResponse {
    /// `<module>.<figure>`.
    #[schema(example = "invoices.issued_total")]
    kind: String,
    /// The module that declares it.
    #[schema(example = "invoices")]
    module: String,
    description: String,
    /// `amount` (sums of money, per currency) or `quantity` (counts).
    #[schema(example = "amount")]
    measure: String,
    /// Dimensions the facts carry, usable in the `dimensions` filter of `GET /facts`.
    dimensions: Vec<String>,
}

impl From<&KindInfo> for KindResponse {
    fn from(info: &KindInfo) -> Self {
        KindResponse {
            kind: info.kind.kind.to_owned(),
            module: info.module.to_owned(),
            description: info.kind.description.to_owned(),
            measure: info.kind.measure.as_str().to_owned(),
            dimensions: info.kind.dimensions.iter().map(|name| (*name).to_owned()).collect(),
        }
    }
}

/// Which report figures to read. Every filter only narrows what you may see; it never widens it.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub(crate) struct FilterQuery {
    /// The fact kind, e.g. `invoices.issued_total` (see `GET /kinds`). Required for the summary.
    pub kind: Option<String>,
    /// First day (inclusive), `YYYY-MM-DD`, in the time zone of the unit each fact belongs to.
    /// Defaults to 29 days before `to`.
    #[param(example = "2026-11-01")]
    pub from: Option<String>,
    /// Last day (inclusive), `YYYY-MM-DD`. Defaults to today (UTC).
    #[param(example = "2026-11-30")]
    pub to: Option<String>,
    /// Only these tenants of your customer tree, comma-separated ids. A tenant outside what you
    /// may see answers 404 `reporting.tenant_not_found`.
    pub tenant_ids: Option<String>,
    /// Only these organization units, comma-separated ids; units outside your scope are left out.
    pub org_ids: Option<String>,
    /// Only the units of these unit groups (`GET /api/v1/organizations/groups`), comma-separated
    /// ids, and only facts of your own tenant (not of the tenants below it). A group grants no
    /// access: its units outside your scope are left out and counted in `excluded_org_count` of
    /// the summary. An unknown group answers 404 `reporting.group_not_found`.
    pub group_ids: Option<String>,
    /// Only facts with these dimension values, comma-separated `name:value` pairs, e.g.
    /// `kind:vat_invoice` (needs `kind`; see `dimensions` in `GET /kinds`).
    #[param(example = "kind:vat_invoice")]
    pub dimensions: Option<String>,
}

/// How to sum report figures.
#[derive(Debug, Default, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub(crate) struct SummaryOptions {
    /// `day` (the default; at most 366 days), `week` (keyed by its Monday), `month` (keyed by its
    /// first day), `tenant` (keyed by the source tenant id), `org` (keyed by the unit id) or
    /// `group` (keyed by the unit group id; needs `group_ids`). With `group`, a unit in two groups
    /// counts in both, so the groups can add up to more than `totals`.
    #[param(example = "day")]
    pub group_by: Option<String>,
    /// Also convert every sum to this ISO 4217 currency, with the exchange rates of your own tenant
    /// on each day. Pairs without a rate are listed in `missing_rates` and left out of `converted`.
    #[param(example = "VND")]
    pub currency: Option<String>,
}

fn ids(field: &str, raw: Option<&str>, problems: &mut Vec<FieldError>) -> Vec<Uuid> {
    let Some(raw) = raw else { return Vec::new() };
    let mut found = Vec::new();
    for part in raw.split(',').map(str::trim).filter(|part| !part.is_empty()) {
        match Uuid::parse_str(part) {
            Ok(id) if !found.contains(&id) => found.push(id),
            Ok(_) => {}
            Err(_) => {
                problems.push(FieldError::new(field, "invalid", "must be comma-separated ids"));
                return Vec::new();
            }
        }
    }
    if found.len() > 200 {
        problems.push(FieldError::new(field, "too_many", "must name at most 200 ids"));
    }
    found
}

fn day(field: &str, raw: Option<&str>, problems: &mut Vec<FieldError>) -> Option<Date> {
    let raw = raw?;
    match Date::parse_from_str(raw.trim(), "%Y-%m-%d") {
        Ok(day) => Some(day),
        Err(_) => {
            problems.push(FieldError::new(field, "invalid", "must be a date `YYYY-MM-DD`"));
            None
        }
    }
}

impl FilterQuery {
    /// Bộ lọc đã kiểm; gom mọi lỗi để báo một lần (D-083).
    pub(crate) fn read(&self, problems: &mut Vec<FieldError>) -> Filter {
        let mut dimensions = BTreeMap::new();
        if let Some(raw) = &self.dimensions {
            for part in raw.split(',').map(str::trim).filter(|part| !part.is_empty()) {
                match part.split_once(':') {
                    Some((name, value)) if !name.is_empty() && !value.is_empty() => {
                        dimensions.insert(name.to_owned(), value.to_owned());
                    }
                    _ => problems.push(FieldError::new(
                        "dimensions",
                        "invalid",
                        "must be comma-separated `name:value` pairs",
                    )),
                }
            }
        }
        Filter {
            kind: self.kind.clone().filter(|kind| !kind.trim().is_empty()),
            from: day("from", self.from.as_deref(), problems),
            to: day("to", self.to.as_deref(), problems),
            tenant_ids: ids("tenant_ids", self.tenant_ids.as_deref(), problems),
            org_ids: ids("org_ids", self.org_ids.as_deref(), problems),
            group_ids: ids("group_ids", self.group_ids.as_deref(), problems),
            dimensions,
        }
    }
}

impl SummaryOptions {
    pub(crate) fn read(&self, filter: Filter, problems: &mut Vec<FieldError>) -> SummaryQuery {
        let group_by = match self.group_by.as_deref() {
            None => GroupBy::Day,
            Some(raw) => GroupBy::parse(raw).unwrap_or_else(|| {
                problems.push(FieldError::new(
                    "group_by",
                    "invalid",
                    "must be `day`, `week`, `month`, `tenant`, `org` or `group`",
                ));
                GroupBy::Day
            }),
        };
        let currency = self.currency.as_deref().and_then(|raw| match Currency::parse(raw) {
            Ok(currency) => Some(currency),
            Err(_) => {
                problems.push(FieldError::new("currency", "invalid", "must be an ISO 4217 code"));
                None
            }
        });
        SummaryQuery { filter, group_by, currency }
    }
}

pub(crate) fn checked<T>(value: T, problems: Vec<FieldError>) -> Result<T, Error> {
    if problems.is_empty() { Ok(value) } else { Err(Error::validation(problems)) }
}

/// The total of one currency (or of a figure without money: `currency` is null).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct CurrencyTotalResponse {
    /// ISO 4217 code; null for figures that count things.
    currency: Option<String>,
    /// Sum in minor units of `currency`, as a base-10 integer string.
    #[schema(example = "1999000")]
    amount: String,
    /// Sum of the counts.
    quantity: i64,
    /// Number of facts.
    count: i64,
}

impl From<CurrencyTotal> for CurrencyTotalResponse {
    fn from(total: CurrencyTotal) -> Self {
        CurrencyTotalResponse {
            currency: total.currency.map(|currency| currency.code().to_owned()),
            amount: total.amount_minor.to_string(),
            quantity: total.quantity,
            count: total.count,
        }
    }
}

/// One group of the summary.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct SummaryGroupResponse {
    /// The group: a day `YYYY-MM-DD` (first day of the week or month), a tenant id, a unit id or a
    /// unit group id; `total` for the totals.
    #[schema(example = "2026-11-18")]
    key: String,
    /// Sums per original currency; money of two currencies is never added together.
    by_currency: Vec<CurrencyTotalResponse>,
    /// All amounts converted to the asked `currency` and added, when you asked for one.
    converted: Option<Money>,
    /// With `group_ids` (on `totals`) or `group_by=group` (on each group): units of the asked
    /// groups outside your scope, left out of the sums. Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    excluded_org_count: Option<u64>,
}

impl From<SummaryGroup> for SummaryGroupResponse {
    fn from(group: SummaryGroup) -> Self {
        SummaryGroupResponse {
            key: group.key,
            by_currency: group.by_currency.into_iter().map(Into::into).collect(),
            converted: group.converted,
            excluded_org_count: group.excluded_org_count,
        }
    }
}

/// A currency and day without an exchange rate to the asked currency.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct MissingRateResponse {
    #[schema(example = "USD")]
    currency: String,
    day: Date,
}

impl From<MissingRate> for MissingRateResponse {
    fn from(missing: MissingRate) -> Self {
        MissingRateResponse { currency: missing.currency, day: missing.day }
    }
}

/// Sums of one fact kind over a period.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct SummaryResponse {
    kind: String,
    from: Date,
    to: Date,
    group_by: String,
    groups: Vec<SummaryGroupResponse>,
    totals: SummaryGroupResponse,
    /// Amounts in these currencies on these days had no rate and are not in `converted`.
    missing_rates: Vec<MissingRateResponse>,
}

impl From<Summary> for SummaryResponse {
    fn from(summary: Summary) -> Self {
        SummaryResponse {
            kind: summary.kind,
            from: summary.from,
            to: summary.to,
            group_by: summary.group_by.as_str().to_owned(),
            groups: summary.groups.into_iter().map(Into::into).collect(),
            totals: summary.totals.into(),
            missing_rates: summary.missing_rates.into_iter().map(Into::into).collect(),
        }
    }
}

/// One report fact.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct FactResponse {
    id: Uuid,
    /// The tenant the fact comes from.
    tenant_id: Uuid,
    /// The organization unit of that tenant; the root unit has the id of the tenant.
    org_id: Uuid,
    kind: String,
    occurred_at: Timestamp,
    /// The day in the time zone of the unit, when the fact was taken.
    local_day: Date,
    /// For figures of money.
    amount: Option<Money>,
    /// For figures that count things.
    quantity: Option<i64>,
    /// Dimension values, e.g. `{"kind": "vat_invoice"}`.
    dimensions: BTreeMap<String, String>,
}

impl TryFrom<fact::Model> for FactResponse {
    type Error = Error;

    fn try_from(row: fact::Model) -> Result<Self, Error> {
        let amount = match (row.amount_minor, row.amount_currency.as_deref()) {
            (Some(minor), Some(currency)) => Some(Money::from_columns(minor, currency)?),
            _ => None,
        };
        Ok(FactResponse {
            id: row.id,
            tenant_id: row.source_tenant_id,
            org_id: row.org_id,
            kind: row.kind,
            occurred_at: row.occurred_at,
            local_day: row.local_day,
            amount,
            quantity: row.quantity,
            dimensions: serde_json::from_str(&row.dimensions).map_err(Error::internal)?,
        })
    }
}

pub(crate) fn fact_page(page: Page<fact::Model>) -> Result<Page<FactResponse>, Error> {
    let Page { data, pagination } = page;
    let data = data.into_iter().map(FactResponse::try_from).collect::<Result<_, _>>()?;
    Ok(Page { data, pagination })
}

/// How far behind the report store is for one tenant.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct LagResponse {
    tenant_id: Uuid,
    /// Facts not yet pushed to the store.
    pending: u64,
    /// When the oldest fact not yet pushed was taken.
    oldest_pending_at: Option<Timestamp>,
    /// When the store last received a fact of this tenant.
    last_received_at: Option<Timestamp>,
}

impl From<Lag> for LagResponse {
    fn from(lag: Lag) -> Self {
        LagResponse {
            tenant_id: lag.tenant_id.as_uuid(),
            pending: lag.pending,
            oldest_pending_at: lag.oldest_pending_at,
            last_received_at: lag.last_received_at,
        }
    }
}

/// Push the kept facts of a tenant to the report store again.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RebuildRequest {
    /// A tenant of your customer tree; omit for your own tenant.
    pub tenant_id: Option<Uuid>,
}

/// How many facts will be pushed again.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct RebuildResponse {
    requeued: u64,
}

/// List the kinds of report figures.
///
/// Requires `reporting.read`. Each module declares the figures its events produce.
#[utoipa::path(
    get,
    path = "/kinds",
    extensions(("x-permissions" = json!(["reporting.read"]))),
    operation_id = "list_report_kinds",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Every fact kind, by name.", body = Data<Vec<KindResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `reporting.read`; code `permission_denied`.", body = Problem),
    )
)]
async fn list_kinds(
    State(service): State<Arc<Reporting>>,
    principal: Principal,
) -> Result<Data<Vec<KindResponse>>, Error> {
    principal.require(&REPORTING_READ)?;
    Ok(Data { data: kind_list(&service) })
}

/// Mọi loại số liệu, theo tên.
pub(crate) fn kind_list(service: &Reporting) -> Vec<KindResponse> {
    let mut kinds: Vec<KindResponse> = service.kinds().iter().map(Into::into).collect();
    kinds.sort_by(|a, b| a.kind.cmp(&b.kind));
    kinds
}

/// Sum a report figure over a period.
///
/// Requires `reporting.read`. With scope `tenant` you see your tenant and every tenant below it
/// (on any database); with a narrower scope only the facts of your units. Reads the daily totals of
/// the store at the root tenant of your customer tree; facts arrive a few seconds after the change.
#[utoipa::path(
    get,
    path = "/summary",
    extensions(("x-permissions" = json!(["reporting.read"]))),
    operation_id = "report_summary",
    security(("bearer" = [])),
    params(FilterQuery, SummaryOptions),
    responses(
        (status = 200, description = "The sums, per group and in total.", body = Data<SummaryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `reporting.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "A tenant in `tenant_ids` is not one you may see; code `reporting.tenant_not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid (see `errors`), the kind is unknown (`reporting.kind_unknown`), or the period is wrong (`reporting.range_invalid`, `reporting.range_too_large`).", body = Problem),
        (status = 503, description = "The report store is under maintenance; code `reporting.store_unavailable`.", body = Problem),
    )
)]
async fn summary(
    State(service): State<Arc<Reporting>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<FilterQuery>,
    Query(options): Query<SummaryOptions>,
) -> Result<Data<SummaryResponse>, Error> {
    principal.require(&REPORTING_READ)?;
    let mut problems = Vec::new();
    let filter = filter.read(&mut problems);
    let query = checked(options.read(filter, &mut problems), problems)?;
    Ok(Data { data: service.summary(&tenant, &principal, &query).await?.into() })
}

/// List report facts.
///
/// Requires `reporting.read`, with the same visibility as the summary. Newest first unless
/// `order=asc`.
#[utoipa::path(
    get,
    path = "/facts",
    extensions(("x-permissions" = json!(["reporting.read"]))),
    operation_id = "list_report_facts",
    security(("bearer" = [])),
    params(FilterQuery, PageQuery),
    responses(
        (status = 200, description = "One page of facts.", body = Page<FactResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `reporting.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "A tenant in `tenant_ids` is not one you may see; code `reporting.tenant_not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid (see `errors`), the kind or a dimension is unknown (`reporting.kind_unknown`, `reporting.dimension_unknown`), or the period is wrong.", body = Problem),
        (status = 503, description = "The report store is under maintenance; code `reporting.store_unavailable`.", body = Problem),
    )
)]
async fn list_facts(
    State(service): State<Arc<Reporting>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<FilterQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<FactResponse>, Error> {
    principal.require(&REPORTING_READ)?;
    let mut problems = Vec::new();
    let filter = filter.read(&mut problems);
    let filter = checked(filter, problems)?;
    let page = PageRequest::try_from(page)?;
    fact_page(service.facts(&tenant, &principal, &filter, &page).await?)
}

/// See how far behind the report store is.
///
/// Requires `reporting.manage`. One line for your tenant and each tenant below it.
#[utoipa::path(
    get,
    path = "/lag",
    extensions(("x-permissions" = json!(["reporting.manage"]))),
    operation_id = "report_lag",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "Facts waiting to be pushed, per tenant.", body = Data<Vec<LagResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `reporting.manage`; code `permission_denied`.", body = Problem),
    )
)]
async fn lag(
    State(service): State<Arc<Reporting>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<LagResponse>>, Error> {
    principal.require(&REPORTING_MANAGE)?;
    let lags = service.lag(&tenant, &principal).await?;
    Ok(Data { data: lags.into_iter().map(Into::into).collect() })
}

/// Push the facts of a tenant to the report store again.
///
/// Requires `reporting.manage`. Marks the facts the tenant still keeps (see
/// `AUTOXIDE_REPORTING_LOCAL_RETENTION_DAYS`) as not pushed; the store never counts a fact twice.
/// Safe to call again.
#[utoipa::path(
    post,
    path = "/rebuild",
    extensions(("x-permissions" = json!(["reporting.manage"]))),
    operation_id = "report_rebuild",
    security(("bearer" = [])),
    request_body = RebuildRequest,
    responses(
        (status = 200, description = "The number of facts that will be pushed again.", body = Data<RebuildResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `reporting.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The tenant is not in your customer tree; code `reporting.tenant_not_found`.", body = Problem),
        (status = 422, description = "The body is invalid; see `errors`.", body = Problem),
    )
)]
async fn rebuild(
    State(service): State<Arc<Reporting>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<RebuildRequest>,
) -> Result<Data<RebuildResponse>, Error> {
    principal.require(&REPORTING_MANAGE)?;
    let requeued = service.rebuild(&tenant, &principal, input.tenant_id).await?;
    Ok(Data { data: RebuildResponse { requeued } })
}
