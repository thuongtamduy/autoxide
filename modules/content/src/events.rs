//! Event module `content` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module
//! (audit ghi "trước / sau"); event đổi bản công khai thêm `published` với đủ dữ liệu để
//! website làm mới cache (D-102). Event ghi CÙNG transaction với thay đổi.

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
    /// Bản công khai đổi: `entry_id`, `kind`, `locale`, `translation_group_id`, `path` mới
    /// (`null` khi không còn công khai) và `stale_paths` — mọi đường dẫn cũ bị ảnh hưởng (kể
    /// cả của trang con).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published: Option<Value>,
}

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
    (EntryCreated, "content.entry_created", "A post or page was created as a draft (also a new translation)."),
    (EntryUpdated, "content.entry_updated", "The draft of a post or page changed."),
    (EntryPublished, "content.entry_published", "A post or page was published, or its published version replaced."),
    (EntryUnpublished, "content.entry_unpublished", "A post or page was taken down; it is a draft again."),
    (EntryArchived, "content.entry_archived", "A post or page was taken down for good; its address answers 410."),
    (EntryDeleted, "content.entry_deleted", "A post or page was deleted."),
    (EntryMoved, "content.entry_moved", "A page moved under another parent; its address and those of its subpages may have changed."),
    (EntryRestored, "content.entry_restored", "A revision was copied back into the draft of a post or page."),
    (ScheduleCreated, "content.schedule_created", "Publishing or unpublishing was scheduled."),
    (ScheduleCanceled, "content.schedule_canceled", "A schedule was canceled."),
    (ScheduleFailed, "content.schedule_failed", "A schedule could not run."),
    (TermChanged, "content.term_changed", "A category or tag was created, changed or deleted."),
    (AuthorChanged, "content.author_changed", "An author profile was created, changed or deleted."),
    (MenuChanged, "content.menu_changed", "A navigation menu was replaced."),
    (RedirectChanged, "content.redirect_changed", "A redirect was created, changed or deleted by hand."),
}

/// Chủ thể (subject) của event về mục `id`.
pub(crate) fn entry_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("content.entry:{id}"))
}

/// Chủ thể của event về tài nguyên `kind` (`term`, `author`, `menu`, `redirect`, `schedule`).
pub(crate) fn subject(kind: &str, id: uuid::Uuid) -> Option<String> {
    Some(format!("content.{kind}:{id}"))
}
