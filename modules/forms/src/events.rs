//! Event module `forms` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module (audit
//! ghi "trước / sau"). Event ghi CÙNG transaction với thay đổi.
//!
//! Event **không mang câu trả lời** hay email của người gửi: module `webhooks` chuyển mọi event
//! ra ngoài, nên payload chỉ có ID, `key` của form, trạng thái và số đếm. Hệ thống ngoài lấy
//! chi tiết qua API có permission.

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
    (FormCreated, "forms.form_created", "A form was created."),
    (FormUpdated, "forms.form_updated", "A form changed (a new `version` when its fields changed)."),
    (FormDeleted, "forms.form_deleted", "A form was deleted, with all its submissions."),
    (SubmissionReceived, "forms.submission_received", "A submission reached the inbox: sent and accepted, or moved out of spam. Spam never raises it. No answers in the payload."),
    (SubmissionStatusChanged, "forms.submission_status_changed", "A submission was marked read, archived, spam or not spam."),
    (SubmissionDeleted, "forms.submission_deleted", "A submission was deleted by hand."),
    (SubmissionsErased, "forms.submissions_erased", "Every submission of one email address was deleted (a data subject request); the address is not in the payload."),
    (SubmissionsExpired, "forms.submissions_expired", "The daily job deleted submissions past their retention time."),
}

/// Chủ thể (subject) của event về form `id`.
pub(crate) fn form_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("forms.form:{id}"))
}

/// Chủ thể của event về bài gửi `id`.
pub(crate) fn submission_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("forms.submission:{id}"))
}
