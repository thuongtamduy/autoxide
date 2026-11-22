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
    (EntryUnarchived, "content.entry_unarchived", "An archived post or page was brought back as a draft."),
    (EntryDeleted, "content.entry_deleted", "A post or page was deleted."),
    (EntryMoved, "content.entry_moved", "A page moved under another parent; its address and those of its subpages may have changed."),
    (EntryRestored, "content.entry_restored", "A revision was copied back into the draft of a post or page."),
    (ReviewRequested, "content.review_requested", "The writer of a draft (often an AI agent) asked a person to review and publish it."),
    (ReviewWithdrawn, "content.review_withdrawn", "A review request was withdrawn."),
    (TranslationRequested, "content.translation_requested", "An AI translation of a post or page was asked for; it runs in the background."),
    (TranslationSucceeded, "content.translation_succeeded", "An AI translation finished; the translation's draft was created or updated (never published)."),
    (TranslationFailed, "content.translation_failed", "An AI translation could not be made; the request records why (`error_code`)."),
    (ScheduleCreated, "content.schedule_created", "Publishing or unpublishing was scheduled."),
    (ScheduleCanceled, "content.schedule_canceled", "A schedule was canceled."),
    (ScheduleFailed, "content.schedule_failed", "A schedule could not run."),
    (TermChanged, "content.term_changed", "A category or tag was created, changed or deleted."),
    (AuthorChanged, "content.author_changed", "An author profile was created, changed or deleted."),
    (MenuChanged, "content.menu_changed", "A navigation menu was replaced."),
    (RedirectChanged, "content.redirect_changed", "A redirect was created, changed or deleted by hand."),
    (PlacementChanged, "content.placement_changed", "A display placement for banners was created, changed or deleted."),
    (ThemeChanged, "content.theme_changed", "The theme of a website (colors, fonts, spacing, logo) was changed."),
    (SharedSectionChanged, "content.shared_section_changed", "A shared section was created, changed (every page using it changes too; `pages` counts them) or deleted."),
    (TypeChanged, "content.type_changed", "A custom content type was created, changed (also switched off) or deleted."),
    (AiDraftReady, "content.ai_draft_ready", "An AI request on a page finished: its result is a new draft (never published); `revision_id` holds it."),
    (BannerChanged, "content.banner_changed", "A banner was created, changed, reordered, archived (also when the promotion it links to ended) or deleted."),
}

/// Chủ thể (subject) của event về mục `id`.
pub(crate) fn entry_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("content.entry:{id}"))
}

/// Chủ thể của event về tài nguyên `kind` (`term`, `author`, `menu`, `redirect`, `schedule`).
pub(crate) fn subject(kind: &str, id: uuid::Uuid) -> Option<String> {
    Some(format!("content.{kind}:{id}"))
}

/// Event của module, kèm câu "hoạt động gần đây" (bước 51 P5) cho những event người dùng cần thấy.
pub(crate) fn with_activity() -> Vec<EventInfo> {
    all()
        .into_iter()
        .map(|event| match event.name() {
            "content.entry_created" => event
                .activity("{actor} created the draft “{title}”", crate::permissions::CONTENT_READ),
            // Stream SSE (bước 40 P5): website / giao diện làm mới khi bài lên.
            "content.entry_published" => event
                .activity("{actor} published “{title}”", crate::permissions::CONTENT_READ)
                .streamable(crate::permissions::CONTENT_READ),
            "content.entry_unpublished" => {
                event.activity("{actor} took down “{title}”", crate::permissions::CONTENT_READ)
            }
            "content.review_requested" => event.activity(
                "{actor} asked for a review of “{title}”",
                crate::permissions::CONTENT_READ,
            ),
            _ => event,
        })
        .collect()
}
