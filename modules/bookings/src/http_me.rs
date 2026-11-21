//! Khách: `/me/bookings` (khách có tài khoản xem lịch của mình), tệp lịch `.ics`, và đường của khách
//! vãng lai — xin mã một lần, huỷ lịch bằng mã (đặt lịch bằng mã ở `POST /public/bookings`).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::actions::Caller;
use crate::booking::Booking;
use crate::entity::service;
use crate::http_bookings::{BookingRecord, BookingsQuery};
use crate::service::BookingsService;

pub(crate) fn routes(service: Arc<BookingsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(my_bookings))
        .routes(routes!(my_calendar))
        .routes(routes!(booking_calendar))
        .routes(routes!(send_phone_code))
        .routes(routes!(guest_cancel))
        .with_state(service)
}

/// A one-time code sent by SMS, to prove a guest owns the phone number.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BookingPhoneCode {
    /// The `challenge_id` from `POST /api/v1/bookings/public/phone-codes`.
    pub challenge_id: Uuid,
    /// The 6 digits received by SMS.
    #[schema(example = "123456")]
    pub code: String,
}

/// The phone number to send a code to.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendBookingCodeRequest {
    /// E.g. `0912345678` or `+84912345678`.
    #[schema(example = "0912345678")]
    phone: String,
}

/// A code was sent.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BookingCodeSent {
    /// Send it back with the code.
    challenge_id: Uuid,
    /// The code works until this moment, once.
    expires_at: Timestamp,
}

/// A guest cancels a booking.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuestCancelRequest {
    /// A fresh code sent to the phone number of the booking.
    phone_code: BookingPhoneCode,
    /// Up to 200 characters.
    reason: Option<String>,
}

/// Tệp lịch iCalendar (RFC 5545) cho một lịch đặt.
fn calendar(row: &Booking, title: &str) -> String {
    let escape = |text: &str| {
        text.replace('\\', "\\\\")
            .replace(';', "\\;")
            .replace(',', "\\,")
            .replace(['\r', '\n'], " ")
    };
    let stamp = |at: Timestamp| at.format("%Y%m%dT%H%M%SZ").to_string();
    let status = match row.status.as_str() {
        "cancelled" | "expired" => "CANCELLED",
        "held" | "pending" => "TENTATIVE",
        _ => "CONFIRMED",
    };
    [
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        "PRODID:-//Autoxide//Bookings//EN".to_owned(),
        "METHOD:PUBLISH".to_owned(),
        "BEGIN:VEVENT".to_owned(),
        format!("UID:{}@bookings.autoxide", row.id),
        format!("DTSTAMP:{}", stamp(row.updated_at)),
        format!("DTSTART:{}", stamp(row.starts_at)),
        format!("DTEND:{}", stamp(row.ends_at)),
        format!("SUMMARY:{}", escape(title)),
        format!("STATUS:{status}"),
        "END:VEVENT".to_owned(),
        "END:VCALENDAR".to_owned(),
    ]
    .join("\r\n")
        + "\r\n"
}

async fn ics(
    service: &BookingsService,
    tenant: &TenantContext,
    row: Booking,
) -> Result<Response, Error> {
    let title = service::Entity::find_by_id(row.service_id)
        .scoped(tenant)
        .one(tenant.database().read())
        .await?
        .map_or_else(|| "Booking".to_owned(), |found| found.name);
    let _ = service;
    let mut response = calendar(&row, &title).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/calendar; charset=utf-8"));
    Ok(response)
}

/// List my bookings.
///
/// Any signed-in customer: the bookings made with this account, newest first.
#[utoipa::path(
    get,
    path = "/me/bookings",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_bookings",
    security(("bearer" = [])),
    params(BookingsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of your bookings.", body = Page<BookingRecord>),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 422, description = "Invalid filter or paging parameters; see `errors`.", body = Problem),
    )
)]
async fn my_bookings(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(filter): Query<BookingsQuery>,
    Query(query): Query<PageQuery>,
) -> Result<Page<BookingRecord>, Error> {
    let page = PageRequest::try_from(query)?;
    BookingRecord::page(service.my_bookings(&tenant, &principal, &filter.into(), &page).await?)
}

/// Download one of my bookings as a calendar file.
///
/// iCalendar (`.ics`) for adding the booking to a calendar app. Only the customer who made it.
#[utoipa::path(
    get,
    path = "/me/bookings/{id}/calendar.ics",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_booking_calendar",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    responses(
        (status = 200, description = "The calendar file.", body = String, content_type = "text/calendar"),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 404, description = "No such booking of yours; code `bookings.booking_not_found`.", body = Problem),
    )
)]
async fn my_calendar(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    let row = service.get_booking(&tenant, Caller::Principal(&principal), id).await?;
    if row.customer_user_id != Some(principal.user_id()) {
        return Err(crate::booking::booking_not_found(id));
    }
    ics(&service, &tenant, row).await
}

/// Download a booking as a calendar file.
///
/// iCalendar (`.ics`). The shop (`bookings.manage`), the staff member of the booking
/// (`bookings.staff`) or the customer.
#[utoipa::path(
    get,
    path = "/bookings/{id}/calendar.ics",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_booking_calendar",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    responses(
        (status = 200, description = "The calendar file.", body = String, content_type = "text/calendar"),
        (status = 401, description = "Not signed in.", body = Problem),
        (status = 404, description = "No such booking that you can see; code `bookings.booking_not_found`.", body = Problem),
    )
)]
async fn booking_calendar(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    let row = service.get_booking(&tenant, Caller::Principal(&principal), id).await?;
    ics(&service, &tenant, row).await
}

/// Send a booking code by SMS (public).
///
/// Guests book and cancel with a code sent to their phone number. A code works once, for 10
/// minutes, and allows 5 tries; a number gets at most 5 codes an hour.
#[utoipa::path(
    post,
    path = "/public/phone-codes",
    extensions(("x-public" = json!(true))),
    operation_id = "send_booking_phone_code",
    security((), ("bearer" = [])),
    request_body = SendBookingCodeRequest,
    responses(
        (status = 200, description = "Sent.", body = Data<BookingCodeSent>),
        (status = 422, description = "Not a phone number; see `errors`.", body = Problem),
        (status = 429, description = "Too many codes for this number; code `bookings.too_many_codes`.", body = Problem),
        (status = 503, description = "The server cannot send SMS; code `sms_disabled`.", body = Problem),
    )
)]
async fn send_phone_code(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    ValidJson(input): ValidJson<SendBookingCodeRequest>,
) -> Result<Data<BookingCodeSent>, Error> {
    let sent = service.send_code(&tenant, &input.phone).await?;
    Ok(Data { data: BookingCodeSent { challenge_id: sent.id, expires_at: sent.expires_at } })
}

/// Cancel a guest booking (public).
///
/// Guests cancel with a fresh code sent to the phone number of the booking. Same rules as for
/// customers: until the start; within the free window the whole deposit is refunded, later the unit
/// keeps its share.
#[utoipa::path(
    post,
    path = "/public/bookings/{id}/cancel",
    extensions(("x-public" = json!(true))),
    operation_id = "cancel_guest_booking",
    security((), ("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the booking.")),
    request_body = GuestCancelRequest,
    responses(
        (status = 200, description = "The booking, now `cancelled`.", body = Data<BookingRecord>),
        (status = 404, description = "No guest booking with this id for this phone number; code `bookings.booking_not_found`.", body = Problem),
        (status = 409, description = "Already over or cancelled (`bookings.invalid_status`), or started (`bookings.too_late_to_cancel`).", body = Problem),
        (status = 422, description = "Wrong, used or expired code (`bookings.code_invalid`).", body = Problem),
    )
)]
async fn guest_cancel(
    State(service): State<Arc<BookingsService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<GuestCancelRequest>,
) -> Result<Data<BookingRecord>, Error> {
    let phone =
        service.verify_code(&tenant, input.phone_code.challenge_id, &input.phone_code.code).await?;
    let row = service.cancel(&tenant, Caller::Guest(&phone), id, input.reason).await?;
    Ok(Data { data: BookingRecord::new(row)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::SignedDuration;

    #[test]
    fn tep_lich_dung_chuan_va_thoat_ky_tu() {
        let at = Timestamp::parse_rfc3339("2026-11-23T02:00:00Z").unwrap();
        let row = Booking {
            id: Uuid::nil(),
            tenant_id: Uuid::nil(),
            org_id: Uuid::nil(),
            service_id: Uuid::nil(),
            resource_id: Uuid::nil(),
            session_id: None,
            seats: 1,
            starts_at: at,
            ends_at: at + SignedDuration::from_hours(1),
            status: "confirmed".to_owned(),
            customer_user_id: None,
            customer_name: "A".to_owned(),
            customer_phone: None,
            customer_email: None,
            note: String::new(),
            locale: None,
            price_minor: 0,
            currency: "VND".to_owned(),
            deposit_minor: 0,
            deposit_status: "none".to_owned(),
            refund_minor: 0,
            payment_id: None,
            hold_expires_at: None,
            reminded_24h_at: None,
            reminded_2h_at: None,
            cancelled_at: None,
            cancel_reason: None,
            created_by: None,
            created_at: at,
            updated_at: at,
        };
        let text = calendar(&row, "Cut, wash; dry");
        assert!(text.contains("DTSTART:20261123T020000Z\r\n"), "{text}");
        assert!(text.contains("DTEND:20261123T030000Z\r\n"));
        assert!(text.contains("SUMMARY:Cut\\, wash\\; dry\r\n"));
        assert!(text.ends_with("END:VCALENDAR\r\n"));
    }
}
