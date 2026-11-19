//! Event module `conversations` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module (audit
//! đọc "trước / sau" từ đó), cộng trường `audience`: người mua và nhân viên được giao của cuộc trò
//! chuyện — stream SSE của kernel đưa event tới họ dù họ không có `conversations.read` ở cả khách
//! (D-20261119-step61-stream-audience). Event KHÔNG mang nội dung tin nhắn, email hay số điện thoại.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::permissions::{CALL, MANAGE, READ};

pub(crate) const THREAD_OPENED: EventInfo =
    EventInfo::new("conversations.thread_opened", "A customer started a conversation with a unit.")
        .streamable(READ)
        .to_audience();
pub(crate) const THREAD_UPDATED: EventInfo = EventInfo::new(
    "conversations.thread_updated",
    "A conversation was read, handed to a person, given an AI draft or handed over as a ticket.",
)
.streamable(READ)
.to_audience();
pub(crate) const THREAD_ASSIGNED: EventInfo =
    EventInfo::new("conversations.thread_assigned", "A conversation got a new assignee.")
        .streamable(READ)
        .to_audience();
pub(crate) const THREAD_CLOSED: EventInfo =
    EventInfo::new("conversations.thread_closed", "A conversation was closed.")
        .streamable(READ)
        .to_audience();
pub(crate) const MESSAGE_CREATED: EventInfo = EventInfo::new(
    "conversations.message_created",
    "A message was added to a conversation (without its text: read it over REST).",
)
.streamable(READ)
.to_audience();
pub(crate) const MESSAGE_RECALLED: EventInfo = EventInfo::new(
    "conversations.message_recalled",
    "The author of a message recalled it within the allowed time.",
)
.streamable(READ)
.to_audience();
pub(crate) const TYPING: EventInfo = EventInfo::new(
    "conversations.typing",
    "Someone is typing in a conversation (only for real-time display; never stored elsewhere).",
)
.streamable(READ)
.to_audience();
pub(crate) const CUSTOMER_BLOCKED: EventInfo = EventInfo::new(
    "conversations.customer_blocked",
    "A unit blocked a customer from writing, or lifted the block.",
);
pub(crate) const REPORT_CREATED: EventInfo = EventInfo::new(
    "conversations.report_created",
    "Someone reported a conversation or a message as abusive.",
)
.streamable(MANAGE);
pub(crate) const REPORT_RESOLVED: EventInfo =
    EventInfo::new("conversations.report_resolved", "A report of abuse was reviewed.");

pub(crate) const CALLBACK_REQUESTED: EventInfo = EventInfo::new(
    "conversations.callback_requested",
    "A customer asked the shop to call them back (without the phone number).",
)
.streamable(CALL)
.to_audience();
pub(crate) const CALL_UPDATED: EventInfo = EventInfo::new(
    "conversations.call_updated",
    "A call of a conversation changed status (without phone numbers).",
)
.streamable(CALL)
.to_audience();
pub(crate) const CALL_ENDED: EventInfo = EventInfo::new(
    "conversations.call_ended",
    "A call ended: direction, outcome, duration and whether it was recorded.",
);
pub(crate) const RECORDING_OPENED: EventInfo =
    EventInfo::new("conversations.recording_opened", "Someone listened to a call recording.");
pub(crate) const RECORDING_DELETED: EventInfo = EventInfo::new(
    "conversations.recording_deleted",
    "A call recording was deleted (on request, at the end of its retention or with the account).",
);
pub(crate) const VOICE_LINE_CHANGED: EventInfo = EventInfo::new(
    "conversations.voice_line_changed",
    "The phone line of a unit was set up or changed (recording on or off).",
);

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
    /// Người được nhận event qua stream dù không có quyền ở cả khách.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub audience: Vec<Uuid>,
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

event!(ThreadOpened, "conversations.thread_opened");
event!(ThreadUpdated, "conversations.thread_updated");
event!(ThreadAssigned, "conversations.thread_assigned");
event!(ThreadClosed, "conversations.thread_closed");
event!(MessageCreated, "conversations.message_created");
event!(MessageRecalled, "conversations.message_recalled");
event!(Typing, "conversations.typing");
event!(CustomerBlocked, "conversations.customer_blocked");
event!(ReportCreated, "conversations.report_created");
event!(ReportResolved, "conversations.report_resolved");
event!(CallbackRequested, "conversations.callback_requested");
event!(CallUpdated, "conversations.call_updated");
event!(CallEnded, "conversations.call_ended");
event!(RecordingOpened, "conversations.recording_opened");
event!(RecordingDeleted, "conversations.recording_deleted");
event!(VoiceLineChanged, "conversations.voice_line_changed");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        THREAD_OPENED,
        THREAD_UPDATED,
        THREAD_ASSIGNED,
        THREAD_CLOSED,
        MESSAGE_CREATED,
        MESSAGE_RECALLED,
        TYPING,
        CUSTOMER_BLOCKED,
        REPORT_CREATED,
        REPORT_RESOLVED,
        CALLBACK_REQUESTED,
        CALL_UPDATED,
        CALL_ENDED,
        RECORDING_OPENED,
        RECORDING_DELETED,
        VOICE_LINE_CHANGED,
    ]
}

/// Chủ thể (subject) của event về cuộc trò chuyện `id`.
pub(crate) fn thread_subject(id: Uuid) -> Option<String> {
    Some(format!("conversations.thread:{id}"))
}
