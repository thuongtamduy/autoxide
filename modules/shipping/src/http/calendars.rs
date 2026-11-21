//! Lịch giao (`shipping.settings.manage`) và ước tính phí giao công khai cho trang sản phẩm.

use autoxide_kernel::time::{Date, Timestamp};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Currency;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::quote::QuoteResponse;
use crate::calendars::{CalendarFields, CalendarView, DayInput, weekdays};
use crate::entity::calendar;
use crate::estimate::EstimateQuery;
use crate::permissions::SETTINGS_MANAGE;
use crate::service::ShippingService;

/// Fields of a delivery calendar; on create `name` is required.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = ShippingCalendarFields)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalendarRequest {
    /// Organization unit owning the calendar (create only).
    org_id: Option<Uuid>,
    name: Option<String>,
    /// IANA time zone; default `Asia/Ho_Chi_Minh`.
    #[schema(example = "Asia/Ho_Chi_Minh")]
    time_zone: Option<String>,
    /// Working days of the week, 1 = Monday … 7 = Sunday; default Monday to Friday.
    working_days: Option<Vec<u8>>,
}

impl From<CalendarRequest> for CalendarFields {
    fn from(input: CalendarRequest) -> Self {
        CalendarFields {
            org_id: input.org_id,
            name: input.name,
            time_zone: input.time_zone,
            working_days: input.working_days,
        }
    }
}

/// A day with other rules than usual.
#[derive(Debug, Deserialize, Serialize, Validate, ToSchema)]
#[schema(as = ShippingCalendarDay)]
#[serde(deny_unknown_fields)]
pub(crate) struct DayRequest {
    /// `closed` (no dispatch: holiday) or `special_hours` (dispatch with its own cut-off).
    kind: String,
    /// `special_hours`: orders before this minute of the day leave that day.
    cutoff_minute: Option<i32>,
    /// E.g. the name of the holiday.
    #[serde(default)]
    note: String,
}

/// A special day of a calendar.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingCalendarDayEntry)]
pub(crate) struct DayBody {
    day: Date,
    kind: String,
    cutoff_minute: Option<i32>,
    note: String,
}

/// A delivery calendar.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingCalendar)]
pub(crate) struct CalendarBody {
    id: Uuid,
    org_id: Uuid,
    name: String,
    time_zone: String,
    /// 1 = Monday … 7 = Sunday.
    working_days: Vec<u8>,
    /// Holidays and special days (only in single-calendar answers).
    days: Vec<DayBody>,
    updated_at: Timestamp,
}

fn body(row: calendar::Model, days: Vec<DayBody>) -> CalendarBody {
    CalendarBody {
        working_days: weekdays(row.working_weekdays),
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        time_zone: row.time_zone,
        days,
        updated_at: row.updated_at,
    }
}

impl From<CalendarView> for CalendarBody {
    fn from(view: CalendarView) -> Self {
        let days = view
            .days
            .into_iter()
            .map(|day| DayBody {
                day: day.day,
                kind: day.kind,
                cutoff_minute: day.cutoff_minute,
                note: day.note,
            })
            .collect();
        body(view.calendar, days)
    }
}

/// List delivery calendars.
#[utoipa::path(
    get,
    path = "/calendars",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "list_shipping_calendars",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of calendars, without their special days.", body = Page<CalendarBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_calendars(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<CalendarBody>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let page = PageRequest::try_from(page)?;
    Ok(service.list_calendars(&tenant, &principal, &page).await?.map(|row| body(row, Vec::new())))
}

/// Create a delivery calendar.
///
/// Needs `shipping.settings.manage` at the unit. Methods use it (`calendar_id`) to compute expected
/// delivery dates and same-day cut-offs.
#[utoipa::path(
    post,
    path = "/calendars",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "create_shipping_calendar",
    security(("bearer" = [])),
    request_body = CalendarRequest,
    responses(
        (status = 201, description = "Created.", body = Data<CalendarBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit; codes `permission_denied`, `org_out_of_scope`, `org_frozen`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_calendar(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CalendarRequest>,
) -> Result<Created<CalendarBody>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let view = service.create_calendar(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/shipping/calendars/{}", view.calendar.id), view.into()))
}

/// Get a delivery calendar with its special days.
#[utoipa::path(
    get,
    path = "/calendars/{id}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "get_shipping_calendar",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the calendar.")),
    responses(
        (status = 200, description = "The calendar.", body = Data<CalendarBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No calendar with this id in your scope; code `shipping.calendar_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_calendar(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CalendarBody>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.get_calendar(&tenant, &principal, id).await?.into() })
}

/// Change a delivery calendar.
#[utoipa::path(
    patch,
    path = "/calendars/{id}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "update_shipping_calendar",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the calendar.")),
    request_body = CalendarRequest,
    responses(
        (status = 200, description = "The calendar after the change.", body = Data<CalendarBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No calendar with this id in your scope; code `shipping.calendar_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn update_calendar(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CalendarRequest>,
) -> Result<Data<CalendarBody>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.update_calendar(&tenant, &principal, id, input.into()).await?.into() })
}

/// Set a special day of a delivery calendar (holiday, other cut-off).
#[utoipa::path(
    put,
    path = "/calendars/{id}/days/{day}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "set_shipping_calendar_day",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the calendar."),
        ("day" = Date, Path, description = "The day, `YYYY-MM-DD`."),
    ),
    request_body = DayRequest,
    responses(
        (status = 200, description = "The calendar after the change.", body = Data<CalendarBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No calendar with this id in your scope; code `shipping.calendar_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_day(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, day)): Path<(Uuid, Date)>,
    ValidJson(input): ValidJson<DayRequest>,
) -> Result<Data<CalendarBody>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let input = DayInput { kind: input.kind, cutoff_minute: input.cutoff_minute, note: input.note };
    Ok(Data { data: service.put_calendar_day(&tenant, &principal, id, day, input).await?.into() })
}

/// Remove a special day of a delivery calendar.
#[utoipa::path(
    delete,
    path = "/calendars/{id}/days/{day}",
    extensions(("x-permissions" = json!(["shipping.settings.manage"]))),
    operation_id = "delete_shipping_calendar_day",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the calendar."),
        ("day" = Date, Path, description = "The day, `YYYY-MM-DD`."),
    ),
    responses(
        (status = 200, description = "The calendar after the change.", body = Data<CalendarBody>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `shipping.settings.manage` at the unit; codes `permission_denied`, `org_frozen`.", body = Problem),
        (status = 404, description = "No calendar with this id in your scope; code `shipping.calendar_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_day(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, day)): Path<(Uuid, Date)>,
) -> Result<Data<CalendarBody>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    Ok(Data { data: service.delete_calendar_day(&tenant, &principal, id, day).await?.into() })
}

/// Parameters of a public shipping estimate.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct EstimateParams {
    /// The variant on the product page.
    variant_id: Uuid,
    /// Its product (checked when given).
    product_id: Option<Uuid>,
    /// Default 1.
    quantity: Option<i64>,
    /// Code of the destination province.
    province_code: String,
    /// ISO country code, default `VN`.
    country: Option<String>,
    /// The selling store when a chain website sells from one store; default: the product's unit.
    org_id: Option<Uuid>,
    /// ISO 4217 currency, default `VND`.
    currency: Option<String>,
    /// Language of method names, default `en`.
    locale: Option<String>,
}

/// A public shipping estimate.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ShippingEstimate)]
pub(crate) struct EstimateBody {
    /// Always `true`: the fee is computed again when the order is placed.
    estimate: bool,
    #[serde(flatten)]
    quote: QuoteResponse,
}

/// Estimate shipping for a product page.
///
/// Public. Fees and expected delivery dates to a province for a quantity of one variant, from the
/// same rates as checkout, with the free-shipping threshold and today's cut-off of the store. Only
/// an estimate: the fee is computed again for the real cart.
#[utoipa::path(
    get,
    path = "/public/estimate",
    operation_id = "estimate_shipping",
    extensions(("x-public" = json!(true))),
    security(()),
    params(EstimateParams),
    responses(
        (status = 200, description = "The options, cheapest first (`estimate: true`). Cached briefly.", body = Data<EstimateBody>),
        (status = 404, description = "No variant on sale with this id; code `shipping.variant_not_for_sale`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`. No catalog installed: `shipping.estimate_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn estimate(
    State(service): State<Arc<ShippingService>>,
    tenant: TenantContext,
    Query(params): Query<EstimateParams>,
) -> Result<axum::response::Response, Error> {
    let query = EstimateQuery {
        variant_id: params.variant_id,
        product_id: params.product_id,
        quantity: params.quantity.unwrap_or(1),
        country: params.country.unwrap_or_else(|| "VN".to_owned()),
        province_code: params.province_code,
        org_id: params.org_id,
        currency: Currency::parse(params.currency.as_deref().unwrap_or("VND"))?,
        locale: params.locale.unwrap_or_else(|| "en".to_owned()),
    };
    let quote = service.estimate(&tenant, query).await?;
    let data = Data { data: EstimateBody { estimate: true, quote: quote.into() } };
    Ok(([(header::CACHE_CONTROL, "public, max-age=60")], data).into_response())
}
