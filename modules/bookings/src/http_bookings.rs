//! Endpoint REST của lịch đặt, dưới `/api/v1/bookings`: danh sách, cửa hàng đặt hộ, xem, xác nhận,
//! check-in, xong, vắng mặt, huỷ, đổi giờ; khách đặt ở `/public/bookings`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::actions::{Action, Caller};
use crate::booking::{Booked, Booking, Customer, NewBooking, Origin};
use crate::listing::BookingFilter;
use crate::service::BookingsService;

pub(crate) fn routes(service: Arc<BookingsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_bookings, create_booking))
        .routes(routes!(get_booking))
        .routes(routes!(confirm_booking))
        .routes(routes!(check_in_booking))
        .routes(routes!(complete_booking))
        .routes(routes!(no_show_booking))
        .routes(routes!(cancel_booking))
        .routes(routes!(reschedule_booking))
        .with_state(service)
}

/// The customer of a booking.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingCustomer {
    /// The customer's account, for signed-in customers.
    pub user_id: Option<Uuid>,
    pub name: String,
    /// E.164, e.g. `+84912345678`.
    pub phone: Option<String>,
    pub email: Option<String>,
}

/// Who the booking is for.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BookingCustomerInput {
    /// 1 to 200 characters.
    #[schema(example = "Nguyen Van A")]
    pub name: String,
    /// Phone number, e.g. `0912345678` or `+84912345678`; stored as E.164.
    pub phone: Option<String>,
    pub email: Option<String>,
}

/// A deposit and where it stands.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingDepositState {
    amount: Money,
    /// `none`, `pending` (waiting for the payment), `paid`, `refund_pending`, `refunded` or `retained`
    /// (kept by the policy).
    status: String,
    /// The part given back (or being given back).
    refund: Money,
    /// The payment at the partner (`GET /api/v1/ledger/payments/{id}`).
    payment_id: Option<Uuid>,
}

/// A booking.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingRecord {
    id: Uuid,
    /// The unit (shop, branch) of the staff member.
    org_id: Uuid,
    service_id: Uuid,
    /// The staff member (or court, room…) booked.
    resource_id: Uuid,
    /// Places taken (classes).
    seats: i32,
    starts_at: Timestamp,
    ends_at: Timestamp,
    /// `held` (waiting for the deposit), `pending` (waiting for the shop), `confirmed`, `checked_in`,
    /// `completed`, `cancelled`, `no_show` or `expired` (the deposit was not paid in time).
    status: String,
    customer: BookingCustomer,
    note: String,
    /// Price of all places.
    price: Money,
    deposit: BookingDepositState,
    /// Until when a `held` booking waits for the deposit.
    hold_expires_at: Option<Timestamp>,
    cancelled_at: Option<Timestamp>,
    cancel_reason: Option<String>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl BookingRecord {
    pub(crate) fn new(row: Booking) -> Result<Self, Error> {
        Ok(BookingRecord {
            price: Money::from_columns(row.price_minor, &row.currency)?,
            deposit: BookingDepositState {
                amount: Money::from_columns(row.deposit_minor, &row.currency)?,
                status: row.deposit_status,
                refund: Money::from_columns(row.refund_minor, &row.currency)?,
                payment_id: row.payment_id,
            },
            id: row.id,
            org_id: row.org_id,
            service_id: row.service_id,
            resource_id: row.resource_id,
            seats: row.seats,
            starts_at: row.starts_at,
            ends_at: row.ends_at,
            status: row.status,
            customer: BookingCustomer {
                user_id: row.customer_user_id,
                name: row.customer_name,
                phone: row.customer_phone,
                email: row.customer_email,
            },
            note: row.note,
            hold_expires_at: row.hold_expires_at,
            cancelled_at: row.cancelled_at,
            cancel_reason: row.cancel_reason,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    pub(crate) fn page(page: Page<Booking>) -> Result<Page<Self>, Error> {
        let data = page.data.into_iter().map(Self::new).collect::<Result<_, _>>()?;
        Ok(Page { data, pagination: page.pagination })
    }
}

/// Where the customer pays the deposit.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingCheckout {
    payment_id: Uuid,
    /// Partner page where the customer pays.
    checkout_url: Option<String>,
    /// QR code payload, when the partner gives one.
    qr_payload: Option<String>,
    /// Pay before this moment, or the place is released.
    expires_at: Timestamp,
}

/// A booking just made, with the deposit checkout when one is due.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingCreated {
    booking: BookingRecord,
    /// Present when the booking is `held`: send the customer here to pay the deposit.
    checkout: Option<BookingCheckout>,
}

impl BookingCreated {
    pub(crate) fn new(booked: Booked) -> Result<Self, Error> {
        let checkout = booked.payment.map(|payment| BookingCheckout {
            payment_id: payment.id,
            checkout_url: payment.checkout_url,
            qr_payload: payment.qr_payload,
            expires_at: booked.booking.hold_expires_at.unwrap_or(payment.expires_at),
        });
        Ok(BookingCreated { booking: BookingRecord::new(booked.booking)?, checkout })
    }
}

/// A booking made by the shop (phone call, walk-in): confirmed at once, no deposit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBookingRequest {
    service_id: Uuid,
    /// Omitted: the first free staff member who gives the service.
    resource_id: Option<Uuid>,
    /// On a 5-minute mark, e.g. a `starts_at` from the availability.
    starts_at: Timestamp,
    /// Places (classes); default 1.
    #[serde(default = "one")]
    seats: i32,
    customer: BookingCustomerInput,
    /// Up to 500 characters. Never put health information here.
    #[serde(default)]
    note: String,
    /// Language of messages to the customer, e.g. `vi`.
    locale: Option<String>,
}

fn one() -> i32 {
    1
}

/// Which bookings to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BookingsQuery {
    /// Only this status.
    status: Option<String>,
    /// Only this staff member (or court, room…).
    resource_id: Option<Uuid>,
    /// Starting at or after this moment.
    from: Option<Timestamp>,
    /// Starting before this moment.
    to: Option<Timestamp>,
}

impl From<BookingsQuery> for BookingFilter {
    fn from(query: BookingsQuery) -> Self {
        BookingFilter {
            status: query.status,
            resource_id: query.resource_id,
            from: query.from,
            to: query.to,
        }
    }
}

/// Why the booking is cancelled.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelBookingRequest {
    /// Up to 200 characters.
    reason: Option<String>,
}

/// The new time of a booking.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RescheduleBookingRequest {
    /// On a 5-minute mark.
    starts_at: Timestamp,
    /// Another staff member who gives the service; omitted: the same one.
    resource_id: Option<Uuid>,
}

impl From<BookingCustomerInput> for Customer {
    fn from(input: BookingCustomerInput) -> Self {
        Customer { user_id: None, name: input.name, phone: input.phone, email: input.email }
    }
}

impl CreateBookingRequest {
    fn into_new(self) -> NewBooking {
        NewBooking {
            service_id: self.service_id,
            resource_id: self.resource_id,
            starts_at: self.starts_at,
            seats: self.seats,
            customer: self.customer.into(),
            note: self.note,
            locale: self.locale,
            payment_method: "card".to_owned(),
            return_url: None,
        }
    }
}

/// List bookings.
///
/// Requires `bookings.manage` or `bookings.read` (every booking of your units), or `bookings.staff`
/// (only the bookings of the resources linked to your account). Newest first by default.
#[utoipa::path(
    get,
    path = "/bookings",
    extensions(("x-permissions" = json!(["bookings.manage", "bookings.read", "bookings.staff"]))),
    operation_id = "list_bookings",
    security(("bearer" = [])),
    params(BookingsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of bookings.", body = Page<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.manage` or `bookings.staff`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid filter or paging parameters; see `errors`.", body = Problem),
    )
)]
async fn list_bookings(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<BookingsQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<BookingRecord>, Error> {
    // Quyền trước tham số trang: thiếu quyền là 403, không phải 422 (luật `permission-first` của xtask).
    principal.require_any(&[
        &crate::permissions::BOOKINGS_MANAGE,
        &crate::permissions::BOOKINGS_READ,
        &crate::permissions::BOOKINGS_STAFF,
    ])?;
    let page = PageRequest::try_from(query)?;
    BookingRecord::page(service.list_bookings(&tenant, &principal, &filter.into(), &page).await?)
}

/// Book for a customer.
///
/// Requires `bookings.manage` at the staff member's unit. For phone calls and walk-ins: the booking
/// is confirmed at once and asks for no deposit. The time must be inside working hours
/// (`bookings.outside_hours`) and free (`bookings.slot_taken`).
#[utoipa::path(
    post,
    path = "/bookings",
    extensions(("x-permissions" = json!(["bookings.manage"]))),
    operation_id = "create_booking",
    security(("bearer" = [])),
    request_body = CreateBookingRequest,
    responses(
        (status = 201, description = "Booked; `Location` is the booking.", body = Data<BookingCreated>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.manage` at the unit.", body = Problem),
        (status = 404, description = "No active service has this id; code `bookings.service_not_found`.", body = Problem),
        (status = 409, description = "The time was taken (`bookings.slot_taken`) or the class is full (`bookings.capacity_full`).", body = Problem),
        (status = 422, description = "Invalid values (see `errors`), outside working hours (`bookings.outside_hours`) or a resource that does not give the service (`bookings.resource_not_offered`).", body = Problem),
    )
)]
async fn create_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateBookingRequest>,
) -> Result<Created<BookingCreated>, Error> {
    principal.require(&crate::permissions::BOOKINGS_MANAGE)?;
    let new = input.into_new();
    let booked = service.book(&tenant, Some(&principal), Origin::Shop, new).await?;
    let id = booked.booking.id;
    Ok(Created::new(format!("/bookings/bookings/{id}"), BookingCreated::new(booked)?))
}

/// Get a booking.
///
/// Any signed-in user may ask; the answer is the booking only for the shop (`bookings.manage`,
/// `bookings.read`), the staff member of the booking (`bookings.staff`) or the customer who made it.
#[utoipa::path(
    get,
    path = "/bookings/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_booking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    responses(
        (status = 200, description = "The booking.", body = Data<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 404, description = "No such booking that you can see; code `bookings.booking_not_found`.", body = Problem),
    )
)]
async fn get_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingRecord>, Error> {
    let row = service.get_booking(&tenant, Caller::Principal(&principal), id).await?;
    Ok(Data { data: BookingRecord::new(row)? })
}

async fn act(
    service: &BookingsService,
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
    action: Action,
) -> Result<Data<BookingRecord>, Error> {
    let row = service.act(tenant, principal, id, action).await?;
    Ok(Data { data: BookingRecord::new(row)? })
}

/// Confirm a pending booking.
///
/// Requires `bookings.manage`. For shops that approve bookings without a deposit
/// (setting `bookings.require_approval`).
#[utoipa::path(
    post,
    path = "/bookings/{id}/confirm",
    extensions(("x-permissions" = json!(["bookings.manage"]))),
    operation_id = "confirm_booking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    responses(
        (status = 200, description = "The booking, now `confirmed`.", body = Data<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such booking; code `bookings.booking_not_found`.", body = Problem),
        (status = 409, description = "The booking is not `pending`; code `bookings.invalid_status`.", body = Problem),
    )
)]
async fn confirm_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingRecord>, Error> {
    act(&service, &tenant, &principal, id, Action::Confirm).await
}

/// Check the customer in.
///
/// Requires `bookings.manage`, or `bookings.staff` for your own appointments.
#[utoipa::path(
    post,
    path = "/bookings/{id}/check-in",
    extensions(("x-permissions" = json!(["bookings.manage", "bookings.staff"]))),
    operation_id = "check_in_booking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    responses(
        (status = 200, description = "The booking, now `checked_in`.", body = Data<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.manage` or `bookings.staff` (`permission_denied`), or not your appointment (`bookings.not_allowed`).", body = Problem),
        (status = 404, description = "No such booking; code `bookings.booking_not_found`.", body = Problem),
        (status = 409, description = "The booking is not `confirmed`; code `bookings.invalid_status`.", body = Problem),
    )
)]
async fn check_in_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingRecord>, Error> {
    act(&service, &tenant, &principal, id, Action::CheckIn).await
}

/// Mark a booking done.
///
/// Requires `bookings.manage`, or `bookings.staff` for your own appointments.
#[utoipa::path(
    post,
    path = "/bookings/{id}/complete",
    extensions(("x-permissions" = json!(["bookings.manage", "bookings.staff"]))),
    operation_id = "complete_booking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    responses(
        (status = 200, description = "The booking, now `completed`.", body = Data<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.manage` or `bookings.staff` (`permission_denied`), or not your appointment (`bookings.not_allowed`).", body = Problem),
        (status = 404, description = "No such booking; code `bookings.booking_not_found`.", body = Problem),
        (status = 409, description = "The booking is not `confirmed` or `checked_in`; code `bookings.invalid_status`.", body = Problem),
    )
)]
async fn complete_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingRecord>, Error> {
    act(&service, &tenant, &principal, id, Action::Complete).await
}

/// Mark a no-show.
///
/// Requires `bookings.manage`, or `bookings.staff` for your own appointments. Only after the start
/// time. The deposit is kept by the unit's policy (`bookings.no_show_retain_percent`); the rest is
/// refunded through the payment partner.
#[utoipa::path(
    post,
    path = "/bookings/{id}/no-show",
    extensions(("x-permissions" = json!(["bookings.manage", "bookings.staff"]))),
    operation_id = "no_show_booking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    responses(
        (status = 200, description = "The booking, now `no_show`.", body = Data<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Missing permission `bookings.manage` or `bookings.staff` (`permission_denied`), or not your appointment (`bookings.not_allowed`).", body = Problem),
        (status = 404, description = "No such booking; code `bookings.booking_not_found`.", body = Problem),
        (status = 409, description = "Not `confirmed` (`bookings.invalid_status`) or not started yet (`bookings.not_started`).", body = Problem),
    )
)]
async fn no_show_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookingRecord>, Error> {
    act(&service, &tenant, &principal, id, Action::NoShow).await
}

/// Cancel a booking.
///
/// The shop (`bookings.manage`) at any time, with the whole deposit refunded; the customer until the
/// start: within the free window (`bookings.free_cancel_hours`) the whole deposit is refunded, later
/// the unit keeps `bookings.late_cancel_retain_percent` of it. Refunds go through the payment partner.
#[utoipa::path(
    post,
    path = "/bookings/{id}/cancel",
    extensions(("x-permissions" = json!([]))),
    operation_id = "cancel_booking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    request_body = CancelBookingRequest,
    responses(
        (status = 200, description = "The booking, now `cancelled`.", body = Data<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Staff and read-only users cannot cancel; code `bookings.not_allowed`.", body = Problem),
        (status = 404, description = "No such booking; code `bookings.booking_not_found`.", body = Problem),
        (status = 409, description = "Already over or cancelled (`bookings.invalid_status`), or started (`bookings.too_late_to_cancel`).", body = Problem),
        (status = 422, description = "Invalid reason; see `errors`.", body = Problem),
    )
)]
async fn cancel_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CancelBookingRequest>,
) -> Result<Data<BookingRecord>, Error> {
    let row = service.cancel(&tenant, Caller::Principal(&principal), id, input.reason).await?;
    Ok(Data { data: BookingRecord::new(row)? })
}

/// Move a booking.
///
/// The shop (`bookings.manage`) at any time; the customer within the free cancellation window. The
/// new time must be inside working hours and free; if it was just taken (`bookings.slot_taken`)
/// the booking keeps its old time.
#[utoipa::path(
    post,
    path = "/bookings/{id}/reschedule",
    extensions(("x-permissions" = json!([]))),
    operation_id = "reschedule_booking",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    request_body = RescheduleBookingRequest,
    responses(
        (status = 200, description = "The booking at its new time.", body = Data<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 403, description = "Staff and read-only users cannot move bookings; code `bookings.not_allowed`.", body = Problem),
        (status = 404, description = "No such booking; code `bookings.booking_not_found`.", body = Problem),
        (status = 409, description = "Taken (`bookings.slot_taken`), class full (`bookings.capacity_full`), wrong status (`bookings.invalid_status`) or too late for the customer (`bookings.too_late_to_change`).", body = Problem),
        (status = 422, description = "Invalid time (see `errors`) or outside working hours (`bookings.outside_hours`).", body = Problem),
    )
)]
async fn reschedule_booking(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<RescheduleBookingRequest>,
) -> Result<Data<BookingRecord>, Error> {
    let row = service
        .reschedule(&tenant, Caller::Principal(&principal), id, input.starts_at, input.resource_id)
        .await?;
    Ok(Data { data: BookingRecord::new(row)? })
}
