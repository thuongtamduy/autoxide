//! Event module `media` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module:
//! module `audit` ghi "trước / sau" từ đó. Event ghi CÙNG transaction với thay đổi.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

/// Một loại event: tên, mô tả, và kiểu dữ liệu `Change`.
macro_rules! events {
    ($(($kind:ident, $name:literal, $description:literal)),* $(,)?) => {
        $(
            #[derive(Debug, Serialize)]
            #[serde(transparent)]
            pub(crate) struct $kind(pub Change);

            impl Event for $kind {
                const NAME: &'static str = $name;
            }
        )*

        pub(crate) fn all() -> Vec<EventInfo> {
            vec![$(EventInfo::new($name, $description)),*]
        }
    };
}

events! {
    (ItemUploaded, "media.item_uploaded", "A media item was uploaded."),
    (ItemUpdated, "media.item_updated", "The details of a media item (name, texts, folder, visibility) changed."),
    (ItemContentReplaced, "media.item_content_replaced", "The content of a media item was replaced; its id stayed."),
    (ItemTrashed, "media.item_trashed", "A media item was moved to the trash."),
    (ItemRestored, "media.item_restored", "A media item was restored from the trash."),
    (ItemPurged, "media.item_purged", "A media item was deleted for good."),
    (FolderCreated, "media.folder_created", "A media folder was created."),
    (FolderUpdated, "media.folder_updated", "A media folder was renamed or moved."),
    (FolderDeleted, "media.folder_deleted", "An empty media folder was deleted."),
}

/// Chủ thể (subject) của event về item `id`.
pub(crate) fn item_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("media.item:{id}"))
}

/// Chủ thể của event về thư mục `id`.
pub(crate) fn folder_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("media.folder:{id}"))
}
