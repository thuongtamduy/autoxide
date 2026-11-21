//! Tool MCP của module `bookings`. Mỗi tool: một kiểu đầu vào (kiểm như thân JSON của REST),
//! permission như endpoint REST tương ứng, thân là MỘT lời gọi service — không có logic riêng (luật
//! kiến trúc 8).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::tenancy::TenantContext;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::actions::Caller;
use crate::booking::{Customer, NewBooking, Origin};
use crate::http_bookings::{BookingCreated, BookingRecord};
use crate::http_public::{AvailabilityParams, BookingAvailability};
use crate::listing::BookingFilter;
use crate::permissions::{BOOKINGS_MANAGE, BOOKINGS_READ};
use crate::service::BookingsService;

/// Which bookings to list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListBookingsInput {
    /// Only this status: `held`, `pending`, `confirmed`, `checked_in`, `completed`, `cancelled`,
    /// `no_show` or `expired`.
    status: Option<String>,
    /// Only this staff member (or court, room…).
    resource_id: Option<Uuid>,
    /// Starting at or after this moment.
    from: Option<Timestamp>,
    /// Starting before this moment.
    to: Option<Timestamp>,
    /// 1 to 200; default 50.
    limit: Option<u32>,
    /// `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

/// A booking made for a customer (phone call, walk-in): confirmed at once, no deposit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBookingInput {
    service_id: Uuid,
    /// Omitted: the first free staff member who gives the service.
    resource_id: Option<Uuid>,
    /// A `starts_at` from `bookings.availability`.
    starts_at: Timestamp,
    /// Places (classes); default 1.
    seats: Option<i32>,
    /// Customer's name, 1 to 200 characters.
    customer_name: String,
    /// Customer's phone, e.g. `0912345678`; give a phone or an email.
    customer_phone: Option<String>,
    /// Customer's email.
    customer_email: Option<String>,
    /// Up to 500 characters; no health information.
    #[serde(default)]
    note: String,
}

/// The new time of a booking.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RescheduleBookingInput {
    /// Id of the booking.
    id: Uuid,
    /// The new start, on a 5-minute mark.
    starts_at: Timestamp,
    /// Another staff member who gives the service; omitted: the same one.
    resource_id: Option<Uuid>,
}

/// A booking to cancel.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CancelBookingInput {
    /// Id of the booking.
    id: Uuid,
    /// Up to 200 characters.
    reason: Option<String>,
}

pub(crate) fn all(service: &Arc<BookingsService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (find, list, create, move_it, cancel) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "availability",
            "Find free start times of a service between `from` and `to` (at most 31 days), with the price, \
             deposit and cancellation terms to tell the customer before booking.",
            move |tenant: TenantContext, _: Principal, input: AvailabilityParams| {
                let service = find.clone();
                async move { BookingAvailability::new(service.availability(&tenant, input.into()).await?) }
            },
        )
        .for_any_signed_in()
        .read_only(),
        Tool::new(
            "list_bookings",
            "List the bookings of your units, newest first; filter by status, staff member and start time.",
            move |tenant: TenantContext, principal: Principal, input: ListBookingsInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, input.order).page()?;
                    let filter = BookingFilter {
                        status: input.status,
                        resource_id: input.resource_id,
                        from: input.from,
                        to: input.to,
                    };
                    BookingRecord::page(service.list_bookings(&tenant, &principal, &filter, &page).await?)
                }
            },
        )
        .requires(BOOKINGS_READ)
        .or_requires(BOOKINGS_MANAGE)
        .read_only(),
        Tool::new(
            "create_booking",
            "Book a free time for a customer (phone call, walk-in). Confirmed at once, no deposit. Fails with \
             `bookings.slot_taken` if the time was just taken.",
            move |tenant: TenantContext, principal: Principal, input: CreateBookingInput| {
                let service = create.clone();
                async move {
                    let new = NewBooking {
                        service_id: input.service_id,
                        resource_id: input.resource_id,
                        starts_at: input.starts_at,
                        seats: input.seats.unwrap_or(1),
                        customer: Customer {
                            user_id: None,
                            name: input.customer_name,
                            phone: input.customer_phone,
                            email: input.customer_email,
                        },
                        note: input.note,
                        locale: None,
                        payment_method: "card".to_owned(),
                        return_url: None,
                    };
                    BookingCreated::new(service.book(&tenant, Some(&principal), Origin::Shop, new).await?)
                }
            },
        )
        .requires(BOOKINGS_MANAGE),
        Tool::new(
            "reschedule_booking",
            "Move a booking to another free time (and optionally another staff member). If the new time was \
             just taken, the booking keeps its old time.",
            move |tenant: TenantContext, principal: Principal, input: RescheduleBookingInput| {
                let service = move_it.clone();
                async move {
                    let caller = Caller::Principal(&principal);
                    let row = service.reschedule(&tenant, caller, input.id, input.starts_at, input.resource_id).await?;
                    BookingRecord::new(row)
                }
            },
        )
        .requires(BOOKINGS_MANAGE),
        Tool::new(
            "cancel_booking",
            "Cancel a booking for the shop: the place is freed and any deposit paid is refunded in full \
             through the payment partner.",
            move |tenant: TenantContext, principal: Principal, input: CancelBookingInput| {
                let service = cancel.clone();
                async move {
                    let row = service.cancel(&tenant, Caller::Principal(&principal), input.id, input.reason).await?;
                    BookingRecord::new(row)
                }
            },
        )
        .requires(BOOKINGS_MANAGE)
        .destructive(),
    ]
}
