//! Endpoint công khai (không cần đăng nhập) dưới `/api/v1/bookings/public`: dịch vụ đang nhận đặt và
//! giờ trống. Giờ trống cache ngắn bằng HTTP (D-20261118-step60-bookings mục 4).

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::openings::{Availability, AvailabilityQuery};
use crate::service::BookingsService;
use crate::setup::ServiceView;

pub(crate) fn routes(service: Arc<BookingsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(public_services))
        .routes(routes!(public_availability))
        .with_state(service)
}

/// Thời gian cache của giờ trống.
const AVAILABILITY_MAX_AGE: u32 = 15;

/// The deposit a booking of this service asks for, and what happens to it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PublicBookingTerms {
    /// Deposit paid when booking, through the shop's licensed payment partner; zero: none.
    deposit: Money,
    /// Cancel at least this many hours before the start to get the whole deposit back.
    free_cancel_hours: i64,
    /// Percent of the deposit kept on a later cancellation.
    late_cancel_retain_percent: i64,
    /// Percent of the deposit kept when the customer does not come.
    no_show_retain_percent: i64,
    /// Bookings without a deposit wait for the shop to confirm them.
    requires_approval: bool,
}

/// A service customers can book.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct PublicBookingService {
    id: Uuid,
    org_id: Uuid,
    name: String,
    names: BTreeMap<String, String>,
    description: String,
    duration_minutes: i32,
    price: Money,
    capacity: i32,
    /// Staff (or courts, rooms…) that can give it; give one as `resource_id` to book that one.
    performers: Vec<Uuid>,
}

impl PublicBookingService {
    fn from_view(view: ServiceView) -> Result<Self, Error> {
        let price = view.price()?;
        let names = view.names();
        Ok(PublicBookingService {
            id: view.row.id,
            org_id: view.row.org_id,
            name: view.row.name,
            names,
            description: view.row.description,
            duration_minutes: view.row.duration_minutes,
            price,
            capacity: view.row.capacity,
            performers: view.performers,
        })
    }
}

/// Which services to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicServicesQuery {
    /// Only services of this unit (a shop or branch).
    org_id: Option<Uuid>,
}

/// A free time.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingOpening {
    /// The staff member (or court, room…) free at this time.
    resource_id: Uuid,
    starts_at: Timestamp,
    ends_at: Timestamp,
    /// `starts_at` in the local time of the unit, with its offset, e.g. `2026-11-23T09:00:00+07:00`.
    local_start: String,
    /// Places left (always 1 for one-to-one services).
    seats_left: i32,
}

/// Free times of a service, with what the customer must know before booking.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingAvailability {
    service_id: Uuid,
    /// IANA time zone of the unit, e.g. `Asia/Ho_Chi_Minh`.
    #[schema(example = "Asia/Ho_Chi_Minh")]
    timezone: String,
    terms: PublicBookingTerms,
    openings: Vec<BookingOpening>,
}

impl BookingAvailability {
    pub(crate) fn new(found: Availability) -> Result<Self, Error> {
        let price = found.service.price()?;
        let deposit = Money::new(found.service.deposit().of(price.minor()), price.currency());
        let zone = found.zone;
        Ok(BookingAvailability {
            service_id: found.service.row.id,
            timezone: zone.iana_name().unwrap_or("UTC").to_owned(),
            terms: PublicBookingTerms {
                deposit,
                free_cancel_hours: found.policy.free_cancel_hours,
                late_cancel_retain_percent: found.policy.late_cancel_retain_percent,
                no_show_retain_percent: found.policy.no_show_retain_percent,
                requires_approval: found.policy.require_approval && deposit.is_zero(),
            },
            openings: found
                .openings
                .into_iter()
                .map(|opening| BookingOpening {
                    resource_id: opening.resource_id,
                    starts_at: opening.starts_at,
                    ends_at: opening.ends_at,
                    local_start: opening.starts_at.to_rfc3339_in(&zone),
                    seats_left: opening.seats_left,
                })
                .collect(),
        })
    }
}

/// Which free times to find.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema, Validate)]
#[into_params(parameter_in = Query)]
pub(crate) struct AvailabilityParams {
    /// The service.
    pub service_id: Uuid,
    /// Only staff of this unit, and its time zone; omitted: the service's unit.
    pub org_id: Option<Uuid>,
    /// Only this staff member (or court, room…); omitted: anyone who gives the service.
    pub resource_id: Option<Uuid>,
    /// Start of the range.
    pub from: Timestamp,
    /// End of the range, at most 31 days after `from`.
    pub to: Timestamp,
}

impl From<AvailabilityParams> for AvailabilityQuery {
    fn from(params: AvailabilityParams) -> Self {
        AvailabilityQuery {
            service_id: params.service_id,
            org_id: params.org_id,
            resource_id: params.resource_id,
            from: params.from,
            to: params.to,
        }
    }
}

/// List bookable services (public).
///
/// Active services, by name (A→Z unless `order=desc`); paged with `limit` / `cursor`. No sign-in
/// needed.
#[utoipa::path(
    get,
    path = "/public/services",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_booking_services",
    security((), ("bearer" = [])),
    params(PublicServicesQuery, NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the services, by name.", body = Page<PublicBookingService>),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn public_services(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    Query(query): Query<PublicServicesQuery>,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<PublicBookingService>, Error> {
    let page = PageRequest::try_from(page)?;
    let views = service.public_services(&tenant, query.org_id, &page).await?;
    let data =
        views.data.into_iter().map(PublicBookingService::from_view).collect::<Result<_, _>>()?;
    Ok(Page { data, pagination: views.pagination })
}

/// Find free times (public).
///
/// Free start times of a service between `from` and `to`, every 15 minutes of local time: the
/// weekly schedule of each staff member, plus extra hours, minus time off, minus what is already
/// booked (with the service's preparation and cleaning time). The answer also gives the price,
/// deposit and cancellation terms to show before booking. Cached for 15 seconds: a time shown can
/// still be taken when you book (`bookings.slot_taken`).
#[utoipa::path(
    get,
    path = "/public/availability",
    extensions(("x-public" = json!(true))),
    operation_id = "get_booking_availability",
    security((), ("bearer" = [])),
    params(AvailabilityParams),
    responses(
        (status = 200, description = "Free times.", body = Data<BookingAvailability>, headers(("Cache-Control" = String, description = "`public, max-age=15`."))),
        (status = 404, description = "No active service has this id; code `bookings.service_not_found`.", body = Problem),
        (status = 422, description = "Invalid range; see `errors`.", body = Problem),
    )
)]
async fn public_availability(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    Query(params): Query<AvailabilityParams>,
) -> Result<Response, Error> {
    let found = service.availability(&tenant, params.into()).await?;
    let mut response = Data { data: BookingAvailability::new(found)? }.into_response();
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(&format!("public, max-age={AVAILABILITY_MAX_AGE}")) {
        headers.insert(header::CACHE_CONTROL, value);
    }
    headers.insert(header::VARY, HeaderValue::from_static("Autoxide-Tenant"));
    Ok(response)
}
