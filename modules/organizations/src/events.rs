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
pub(crate) const SETTING_CHANGED: EventInfo = EventInfo::new(
    "organizations.setting_changed",
    "A unit set or cleared a setting; units below that do not set it themselves follow.",
);
pub(crate) const PUBLIC_PROFILE_UPDATED: EventInfo = EventInfo::new(
    "organizations.public_profile_updated",
    "The public profile of a unit changed (shown on websites: name, address, opening hours, pickup).",
);
pub(crate) const UNIT_TYPE_CHANGED: EventInfo = EventInfo::new(
    "organizations.unit_type_changed",
    "A unit type was created, renamed or deleted.",
);
pub(crate) const GROUP_CREATED: EventInfo =
    EventInfo::new("organizations.group_created", "A unit group for reports was created.");
pub(crate) const GROUP_UPDATED: EventInfo = EventInfo::new(
    "organizations.group_updated",
    "A unit group was renamed or its description changed.",
);
pub(crate) const GROUP_DELETED: EventInfo =
    EventInfo::new("organizations.group_deleted", "A unit group was deleted (report facts stay).");
pub(crate) const GROUP_MEMBERS_CHANGED: EventInfo = EventInfo::new(
    "organizations.group_members_changed",
    "Units were added to or removed from a unit group (`added`, `removed`).",
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
event!(SettingChanged, "organizations.setting_changed");
event!(PublicProfileUpdated, "organizations.public_profile_updated");
event!(GroupCreated, "organizations.group_created");
event!(GroupUpdated, "organizations.group_updated");
event!(GroupDeleted, "organizations.group_deleted");
event!(GroupMembersChanged, "organizations.group_members_changed");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        UNIT_CREATED,
        UNIT_UPDATED,
        UNIT_MOVED,
        UNIT_STATUS_CHANGED,
        UNIT_TYPE_CHANGED,
        SETTING_CHANGED,
        PUBLIC_PROFILE_UPDATED,
        GROUP_CREATED,
        GROUP_UPDATED,
        GROUP_DELETED,
        GROUP_MEMBERS_CHANGED,
    ]
}

pub(crate) fn unit_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("organizations.unit:{id}"))
}

pub(crate) fn type_subject(code: &str) -> Option<String> {
    Some(format!("organizations.unit_type:{code}"))
}
