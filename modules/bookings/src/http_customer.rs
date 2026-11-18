//! Khách tự đặt lịch (`/api/v1/bookings/public/bookings`): khách có tài khoản đặt bằng phiên đăng
//! nhập; có cọc thì lịch được giữ tạm và câu trả lời mang trang trả cọc của đối tác.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::booking::{Customer, NewBooking, Origin};
use crate::http_bookings::{BookingCreated, BookingCustomerInput};
use crate::service::BookingsService;

pub(crate) fn routes(service: Arc<BookingsService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(book)).with_state(service)
}

/// A booking by the customer.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CustomerBookingRequest {
    service_id: Uuid,
    /// A staff member (or court, room…) from the availability; omitted: the first one free.
    resource_id: Option<Uuid>,
    /// A `starts_at` from the availability.
    starts_at: DateTime<Utc>,
    /// Places (classes); default 1.
    #[serde(default = "one")]
    seats: i32,
    customer: BookingCustomerInput,
    /// Up to 500 characters. Never put health information here.
    #[serde(default)]
    note: String,
    /// Language of messages to the customer, e.g. `vi`.
    locale: Option<String>,
    /// How to pay the deposit, e.g. `card`, `wallet` or `vietqr`; default `card`.
    payment_method: Option<String>,
    /// Page of the shop to open after paying the deposit.
    return_url: Option<String>,
}

fn one() -> i32 {
    1
}

/// Book a service (customer).
///
/// Sign in, then book a free time from `GET /api/v1/bookings/public/availability`. Read the terms
/// there first: price, deposit, cancellation. Without a deposit the booking is `confirmed` (or
/// `pending` when the shop approves bookings); with a deposit it is `held` for a few minutes and the
/// answer has a `checkout` to pay through the shop's licensed payment partner — unpaid holds are
/// released. Two people booking the same time: one gets it, the other `bookings.slot_taken`.
#[utoipa::path(
    post,
    path = "/public/bookings",
    extensions(("x-public" = json!(true))),
    operation_id = "book_service",
    security(("bearer" = [])),
    request_body = CustomerBookingRequest,
    responses(
        (status = 201, description = "Booked; `Location` is the booking.", body = Data<BookingCreated>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 404, description = "No active service has this id; code `bookings.service_not_found`.", body = Problem),
        (status = 409, description = "The time was just taken (`bookings.slot_taken`) or the class is full (`bookings.capacity_full`).", body = Problem),
        (status = 422, description = "Invalid values (see `errors`), outside working hours (`bookings.outside_hours`) or a resource that does not give the service (`bookings.resource_not_offered`).", body = Problem),
        (status = 503, description = "The service asks for a deposit but payments are not set up; code `bookings.payments_unavailable`.", body = Problem),
    )
)]
async fn book(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CustomerBookingRequest>,
) -> Result<Created<BookingCreated>, Error> {
    let customer = Customer {
        user_id: Some(principal.user_id()),
        name: input.customer.name,
        phone: input.customer.phone,
        email: input.customer.email,
    };
    let new = NewBooking {
        service_id: input.service_id,
        resource_id: input.resource_id,
        starts_at: input.starts_at,
        seats: input.seats,
        customer,
        note: input.note,
        locale: input.locale,
        payment_method: input.payment_method.unwrap_or_else(|| "card".to_owned()),
        return_url: input.return_url,
    };
    let booked = service.book(&tenant, Some(&principal), Origin::Customer, new).await?;
    let id = booked.booking.id;
    Ok(Created::new(format!("/bookings/bookings/{id}"), BookingCreated::new(booked)?))
}
