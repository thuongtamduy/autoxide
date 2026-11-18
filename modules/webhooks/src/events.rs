//! Event module `webhooks` phát, cùng hình dạng `{"before": …, "after": …}` với mọi
//! module. Không bao giờ đặt secret của endpoint vào event.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const ENDPOINT_CREATED: EventInfo =
    EventInfo::new("webhooks.endpoint_created", "A webhook endpoint was created.");
pub(crate) const ENDPOINT_UPDATED: EventInfo = EventInfo::new(
    "webhooks.endpoint_updated",
    "A webhook endpoint's address, events or state changed, or its secret was replaced.",
);
pub(crate) const ENDPOINT_DELETED: EventInfo =
    EventInfo::new("webhooks.endpoint_deleted", "A webhook endpoint was deleted.");
pub(crate) const DELIVERY_REDELIVERED: EventInfo =
    EventInfo::new("webhooks.delivery_redelivered", "A webhook delivery was sent again by hand.");

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

macro_rules! event {
    ($kind:ident, $name:literal) => {
        #[derive(Debug, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $kind(pub Change);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

event!(EndpointCreated, "webhooks.endpoint_created");
event!(EndpointUpdated, "webhooks.endpoint_updated");
event!(EndpointDeleted, "webhooks.endpoint_deleted");
event!(DeliveryRedelivered, "webhooks.delivery_redelivered");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![ENDPOINT_CREATED, ENDPOINT_UPDATED, ENDPOINT_DELETED, DELIVERY_REDELIVERED]
}

/// Chủ thể (subject) của event về endpoint `id`.
pub(crate) fn endpoint_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("webhooks.endpoint:{id}"))
}
