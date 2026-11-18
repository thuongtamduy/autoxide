//! Event module `bookings` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module (audit
//! ghi "trước / sau" từ đó). Event ghi CÙNG transaction với thay đổi.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

/// Một loại event: hằng `EventInfo` + kiểu mang `Change`.
macro_rules! events {
    ($( $info:ident, $kind:ident, $name:literal, $description:literal; )*) => {
        $(
            pub(crate) const $info: EventInfo = EventInfo::new($name, $description);

            #[derive(Debug, Serialize)]
            #[serde(transparent)]
            pub(crate) struct $kind(pub Change);

            impl Event for $kind {
                const NAME: &'static str = $name;
            }
        )*

        pub(crate) fn all() -> Vec<EventInfo> {
            vec![$($info),*]
        }
    };
}

events! {
    SERVICE_CREATED, ServiceCreated, "bookings.service_created", "A bookable service was created.";
    SERVICE_UPDATED, ServiceUpdated, "bookings.service_updated", "A bookable service changed.";
    RESOURCE_CREATED, ResourceCreated, "bookings.resource_created", "A staff member, room or equipment was added.";
    RESOURCE_UPDATED, ResourceUpdated, "bookings.resource_updated", "A staff member, room or equipment changed.";
    SCHEDULE_REPLACED, ScheduleReplaced, "bookings.schedule_replaced", "The weekly schedule of a resource was replaced.";
    EXCEPTION_CREATED, ExceptionCreated, "bookings.exception_created", "Time off or an extra shift was added.";
    EXCEPTION_DELETED, ExceptionDeleted, "bookings.exception_deleted", "Time off or an extra shift was removed.";
    BOOKING_CREATED, BookingCreated, "bookings.booking_created", "A booking was made (held, pending or confirmed).";
    BOOKING_CONFIRMED, BookingConfirmed, "bookings.booking_confirmed", "A booking was confirmed (deposit paid or approved).";
    BOOKING_CANCELLED, BookingCancelled, "bookings.booking_cancelled", "A booking was cancelled.";
    BOOKING_RESCHEDULED, BookingRescheduled, "bookings.booking_rescheduled", "A booking moved to another time or resource.";
    BOOKING_CHECKED_IN, BookingCheckedIn, "bookings.booking_checked_in", "The customer arrived.";
    BOOKING_NO_SHOW, BookingNoShow, "bookings.booking_no_show", "The customer did not come.";
    BOOKING_COMPLETED, BookingCompleted, "bookings.booking_completed", "The service was given.";
    BOOKING_EXPIRED, BookingExpired, "bookings.booking_expired", "A held booking was released because the deposit was not paid in time.";
}

pub(crate) fn service_subject(id: Uuid) -> Option<String> {
    Some(format!("bookings.service:{id}"))
}

pub(crate) fn resource_subject(id: Uuid) -> Option<String> {
    Some(format!("bookings.resource:{id}"))
}

pub(crate) fn booking_subject(id: Uuid) -> Option<String> {
    Some(format!("bookings.booking:{id}"))
}
