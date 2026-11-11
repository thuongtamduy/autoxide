//! Event module `notes` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module:
//! module `audit` ghi "trước / sau" từ đó. Event ghi CÙNG transaction với thay đổi.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const NOTE_CREATED: EventInfo =
    EventInfo::new("notes.note_created", "A note was created.");
pub(crate) const NOTE_UPDATED: EventInfo =
    EventInfo::new("notes.note_updated", "A note's title or body changed.");
pub(crate) const NOTE_DELETED: EventInfo =
    EventInfo::new("notes.note_deleted", "A note was deleted.");

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

event!(NoteCreated, "notes.note_created");
event!(NoteUpdated, "notes.note_updated");
event!(NoteDeleted, "notes.note_deleted");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![NOTE_CREATED, NOTE_UPDATED, NOTE_DELETED]
}

/// Chủ thể (subject) của event về ghi chú `id`.
pub(crate) fn note_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("notes.note:{id}"))
}
