//! Route REST của tỉ giá (dưới `/api/v1/ledger`, D-237, D-352): danh sách, tỉ giá hiệu lực một
//! ngày, ghi tay, lấy từ nguồn tỉ giá.

use autoxide_kernel::time::Date;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::decimal::DecimalString;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use validator::Validate;

use crate::api::ExchangeRate;
use crate::fx::RateInput;
use crate::service::LedgerService;

pub(crate) fn routes(service: Arc<LedgerService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_rates, record_rate))
        .routes(routes!(effective_rate))
        .routes(routes!(fetch_rates))
        .with_state(service)
}

/// Which rates.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RatesQuery {
    /// Only rates of this base currency, e.g. `USD`.
    base: Option<String>,
    /// Only rates into this currency, e.g. `VND`.
    quote: Option<String>,
}

/// List recorded exchange rates.
///
/// Requires `ledger.read`. Oldest first by default (`order=desc` for newest first).
#[utoipa::path(
    get,
    path = "/exchange-rates",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "list_ledger_exchange_rates",
    security(("bearer" = [])),
    params(RatesQuery, PageQuery),
    responses(
        (status = 200, description = "One page of exchange rates.", body = Page<ExchangeRate>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn list_rates(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<RatesQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ExchangeRate>, Error> {
    let page = PageRequest::try_from(page)?;
    service.list_rates(&tenant, &principal, filter.base, filter.quote, &page).await
}

/// An exchange rate to record.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerExchangeRateRequest)]
pub(crate) struct RateRequest {
    /// ISO 4217 code of the currency being priced, e.g. `USD`.
    #[schema(example = "USD")]
    base: String,
    /// ISO 4217 code of the currency the price is in, e.g. `VND`.
    #[schema(example = "VND")]
    quote: String,
    /// Units of `quote` for one unit of `base`, as a decimal string (never a JSON number).
    #[schema(value_type = String, example = "25345.5")]
    rate: DecimalString,
    /// First day the rate is used, `YYYY-MM-DD`.
    effective_date: Date,
}

/// Record an exchange rate by hand.
///
/// Requires `ledger.settings_manage` for the whole tenant (rates are shared by every unit). The rate
/// is used from `effective_date` until a later rate of the same pair. Recorded rates are never
/// changed: documents and entries keep the rate they used. Sending the same rate again returns it
/// (200); a different rate for the same pair and day is refused.
#[utoipa::path(
    post,
    path = "/exchange-rates",
    extensions(("x-permissions" = json!(["ledger.settings_manage"]))),
    operation_id = "record_ledger_exchange_rate",
    security(("bearer" = [])),
    request_body = RateRequest,
    responses(
        (status = 200, description = "The same rate was recorded before.", body = Data<ExchangeRate>),
        (status = 201, description = "The rate.", body = Data<ExchangeRate>, headers(("Location" = String, description = "Path of the rate list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.settings_manage` for the whole tenant (`permission_denied` or `org_out_of_scope`).", body = Problem),
        (status = 409, description = "Another rate is recorded for this pair and day; code `ledger.exchange_rate_exists`.", body = Problem),
        (status = 422, description = "A value is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn record_rate(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<RateRequest>,
) -> Result<Response, Error> {
    let outcome = service
        .record_rate(
            &tenant,
            &principal,
            RateInput {
                base: input.base,
                quote: input.quote,
                rate: input.rate.0,
                effective_date: input.effective_date,
            },
        )
        .await?;
    Ok(match outcome {
        Submitted::Created(rate) => {
            Created::new("/ledger/exchange-rates".to_owned(), rate).into_response()
        }
        Submitted::Replayed(rate) => Data { data: rate }.into_response(),
    })
}

/// The pair and day of a rate.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EffectiveQuery {
    /// e.g. `USD`.
    base: String,
    /// e.g. `VND`.
    quote: String,
    /// The day, `YYYY-MM-DD`.
    on: Date,
}

/// Get the exchange rate in force on a day.
///
/// Requires `ledger.read`. The rate with the latest `effective_date` on or before `on`; on the same
/// day a rate entered by hand wins over a fetched one. This is the rate foreign-currency documents
/// and entries of that day use (they keep a copy of it).
#[utoipa::path(
    get,
    path = "/exchange-rates/effective",
    extensions(("x-permissions" = json!(["ledger.read"]))),
    operation_id = "get_ledger_effective_exchange_rate",
    security(("bearer" = [])),
    params(EffectiveQuery),
    responses(
        (status = 200, description = "The rate in force.", body = Data<ExchangeRate>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No rate of this pair on or before that day; code `ledger.exchange_rate_not_found`.", body = Problem),
        (status = 422, description = "A currency code is invalid; every problem in `errors`.", body = Problem),
    )
)]
async fn effective_rate(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<EffectiveQuery>,
) -> Result<Data<ExchangeRate>, Error> {
    let rate =
        service.effective_rate(&tenant, &principal, &query.base, &query.quote, query.on).await?;
    Ok(Data { data: rate })
}

/// Rates to fetch from a rate source.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = LedgerExchangeRateFetch)]
pub(crate) struct FetchRequest {
    /// Name of the rate source set up on this server; omit when there is only one.
    provider: Option<String>,
    /// Currency to price, e.g. `USD`.
    #[schema(example = "USD")]
    base: String,
    /// The day, `YYYY-MM-DD`; omit for today in the tenant's time zone.
    on: Option<Date>,
}

/// Fetch exchange rates from a rate source.
///
/// Requires `ledger.settings_manage` for the whole tenant. Asks the source for the rates of `base`
/// on that day and records them (source `provider:<name>`); rates already recorded are kept.
#[utoipa::path(
    post,
    path = "/exchange-rates/fetch",
    extensions(("x-permissions" = json!(["ledger.settings_manage"]))),
    operation_id = "fetch_ledger_exchange_rates",
    security(("bearer" = [])),
    request_body = FetchRequest,
    responses(
        (status = 200, description = "The rates recorded from the source.", body = Data<Vec<ExchangeRate>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `ledger.settings_manage` for the whole tenant (`permission_denied` or `org_out_of_scope`).", body = Problem),
        (status = 422, description = "`base` is invalid, or no such rate source is set up (`ledger.rate_source_not_configured`).", body = Problem),
        (status = 503, description = "The rate source did not answer; try again later (`payment.provider_unavailable`).", body = Problem),
    )
)]
async fn fetch_rates(
    State(service): State<Arc<LedgerService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<FetchRequest>,
) -> Result<Data<Vec<ExchangeRate>>, Error> {
    let rates =
        service.fetch_rates(&tenant, &principal, input.provider, &input.base, input.on).await?;
    Ok(Data { data: rates })
}
