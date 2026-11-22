//! Events the module emits, in the shape every module uses: `{"before": …, "after": …}`. The
//! official `audit` module records them; `webhooks` delivers them. An event is written in the
//! SAME transaction as the change it describes.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const BOOKMARK_CREATED: EventInfo =
    EventInfo::new("bookmarks.bookmark_created", "A bookmark was created.");
pub(crate) const BOOKMARK_DELETED: EventInfo =
    EventInfo::new("bookmarks.bookmark_deleted", "A bookmark was deleted.");

#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub(crate) struct BookmarkCreated(pub Change);

impl Event for BookmarkCreated {
    const NAME: &'static str = "bookmarks.bookmark_created";
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub(crate) struct BookmarkDeleted(pub Change);

impl Event for BookmarkDeleted {
    const NAME: &'static str = "bookmarks.bookmark_deleted";
}

pub(crate) fn all() -> Vec<EventInfo> {
    vec![BOOKMARK_CREATED, BOOKMARK_DELETED]
}

/// The subject of an event about bookmark `id`: `<kind>:<id>`.
pub(crate) fn subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("bookmarks.bookmark:{id}"))
}
