//! Event module `files` phát, cùng hình dạng `{"before": …, "after": …}` với các module
//! khác để audit log ghi "trước / sau".

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const FILE_UPLOADED: EventInfo =
    EventInfo::new("files.file_uploaded", "A file was uploaded; `after` holds its details.");
pub(crate) const FILE_DELETED: EventInfo =
    EventInfo::new("files.file_deleted", "A file was deleted; `before` holds its details.");

#[derive(Debug, Serialize)]
pub(crate) struct FileUploaded {
    pub after: Value,
}

impl Event for FileUploaded {
    const NAME: &'static str = "files.file_uploaded";
}

#[derive(Debug, Serialize)]
pub(crate) struct FileDeleted {
    pub before: Value,
}

impl Event for FileDeleted {
    const NAME: &'static str = "files.file_deleted";
}

pub(crate) fn all() -> Vec<EventInfo> {
    vec![FILE_UPLOADED, FILE_DELETED]
}

/// Chủ thể (subject) của event về file `id`.
pub(crate) fn file_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("files.file:{id}"))
}
