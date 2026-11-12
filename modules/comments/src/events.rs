//! Event module `comments` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module (audit
//! ghi "trước / sau"). Event ghi CÙNG transaction với thay đổi.
//!
//! Event **không mang chữ bình luận, tên hay email**: module `webhooks` chuyển mọi event ra ngoài,
//! nên payload chỉ có ID, đích, trạng thái và số đếm — đủ để website làm mới cache. Hệ thống
//! ngoài lấy chi tiết qua API có permission.

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
    (CommentCreated, "comments.comment_created", "A comment was stored, with its status (`published`, `pending` or `spam`). No text, name or email in the payload."),
    (CommentPublished, "comments.comment_published", "A comment became visible: posted without approval, or approved."),
    (CommentStatusChanged, "comments.comment_status_changed", "A comment was approved, rejected, marked as spam, or held again after reports."),
    (CommentEdited, "comments.comment_edited", "An author changed a comment (it may be held for approval again)."),
    (CommentDeleted, "comments.comment_deleted", "A comment was deleted: by its author (`hard: false`, its place in the thread stays) or for good by a moderator (`hard: true`, with its replies when it starts a thread)."),
    (TargetOpened, "comments.target_opened", "A target takes comments again."),
    (TargetClosed, "comments.target_closed", "A target stopped taking comments (by hand, or because its owner archived or deleted it); `hidden: true` also hides its comments."),
    (BanCreated, "comments.ban_created", "A person was banned from commenting (by user, email or network fingerprint; the value is not in the payload)."),
    (BanDeleted, "comments.ban_deleted", "A ban was lifted."),
    (CommentsErased, "comments.comments_erased", "Every comment written with one email address was cleared (a data subject request); the address is not in the payload."),
    (CommentsExpired, "comments.comments_expired", "The daily job deleted comments that waited for approval or were spam for too long."),
}

/// Chủ thể (subject) của event về bình luận `id`.
pub(crate) fn comment_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("comments.comment:{id}"))
}

/// Chủ thể của event về đích (`target_type`, `target_id`).
pub(crate) fn target_subject(kind: &str, id: &str) -> Option<String> {
    Some(format!("comments.target:{kind}:{id}"))
}

/// Chủ thể của event về lệnh cấm `id`.
pub(crate) fn ban_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("comments.ban:{id}"))
}
