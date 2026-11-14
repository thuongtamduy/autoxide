//! Event module `menus` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module:
//! module `audit` ghi "trước / sau" từ đó. Event ghi CÙNG transaction với thay đổi.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const MENU_CREATED: EventInfo =
    EventInfo::new("menus.menu_created", "A website menu was created.");
pub(crate) const MENU_REPLACED: EventInfo =
    EventInfo::new("menus.menu_replaced", "The items of a website menu were replaced.");
pub(crate) const MENU_DELETED: EventInfo =
    EventInfo::new("menus.menu_deleted", "A website menu was deleted.");

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

/// Một loại event: tên + dữ liệu `Change`.
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

event!(MenuCreated, "menus.menu_created");
event!(MenuReplaced, "menus.menu_replaced");
event!(MenuDeleted, "menus.menu_deleted");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![MENU_CREATED, MENU_REPLACED, MENU_DELETED]
}

/// Chủ thể (subject) của event về menu `id`.
pub(crate) fn menu_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("menus.menu:{id}"))
}
