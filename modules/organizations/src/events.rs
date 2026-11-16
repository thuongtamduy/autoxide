//! Event module `organizations` phát, cùng hình dạng `{"before": …, "after": …}` với mọi module.
//! Ghi CÙNG transaction với thay đổi.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const UNIT_CREATED: EventInfo =
    EventInfo::new("organizations.unit_created", "An organization unit was created.");
pub(crate) const UNIT_UPDATED: EventInfo = EventInfo::new(
    "organizations.unit_updated",
    "An organization unit was renamed or its code changed.",
);
pub(crate) const UNIT_MOVED: EventInfo = EventInfo::new(
    "organizations.unit_moved",
    "An organization unit, with everything below it, moved under another parent.",
);
pub(crate) const UNIT_STATUS_CHANGED: EventInfo = EventInfo::new(
    "organizations.unit_status_changed",
    "An organization unit was approved, suspended, archived or reactivated.",
);
pub(crate) const UNIT_TYPE_CHANGED: EventInfo = EventInfo::new(
    "organizations.unit_type_changed",
    "A unit type was created, renamed or deleted.",
);

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

event!(UnitCreated, "organizations.unit_created");
event!(UnitUpdated, "organizations.unit_updated");
event!(UnitMoved, "organizations.unit_moved");
event!(UnitStatusChanged, "organizations.unit_status_changed");
event!(UnitTypeChanged, "organizations.unit_type_changed");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![UNIT_CREATED, UNIT_UPDATED, UNIT_MOVED, UNIT_STATUS_CHANGED, UNIT_TYPE_CHANGED]
}

pub(crate) fn unit_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("organizations.unit:{id}"))
}

pub(crate) fn type_subject(code: &str) -> Option<String> {
    Some(format!("organizations.unit_type:{code}"))
}
