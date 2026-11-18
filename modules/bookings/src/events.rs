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
}

pub(crate) fn service_subject(id: Uuid) -> Option<String> {
    Some(format!("bookings.service:{id}"))
}

pub(crate) fn resource_subject(id: Uuid) -> Option<String> {
    Some(format!("bookings.resource:{id}"))
}
