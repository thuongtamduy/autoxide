//! Event module `support` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module (audit
//! đọc "trước / sau" từ đó). Event KHÔNG mang email, số điện thoại, token hay nội dung tin nhắn —
//! chỉ ID, trạng thái, loại, tác giả.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

pub(crate) const TICKET_CREATED: EventInfo =
    EventInfo::new("support.ticket_created", "A support ticket was opened.");
pub(crate) const TICKET_STATUS_CHANGED: EventInfo = EventInfo::new(
    "support.ticket_status_changed",
    "A support ticket changed status (open, pending, solved, closed).",
);
pub(crate) const TICKET_UPDATED: EventInfo = EventInfo::new(
    "support.ticket_updated",
    "The priority, category or assignee of a support ticket changed.",
);
pub(crate) const TICKET_ASSIGNED: EventInfo =
    EventInfo::new("support.ticket_assigned", "A support ticket got a new assignee or team.");
pub(crate) const MESSAGE_ADDED: EventInfo = EventInfo::new(
    "support.message_added",
    "A message, internal note or system line was added to a support ticket (without its text).",
);
pub(crate) const MESSAGE_HIDDEN: EventInfo =
    EventInfo::new("support.message_hidden", "A message of a support ticket was hidden.");
pub(crate) const SETTINGS_CHANGED: EventInfo = EventInfo::new(
    "support.settings_changed",
    "Support teams, team members or routing rules changed.",
);
pub(crate) const TICKETS_CLAIMED: EventInfo = EventInfo::new(
    "support.tickets_claimed",
    "Guest tickets were attached to the account that verified the same email or phone number.",
);

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
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

event!(TicketCreated, "support.ticket_created");
event!(TicketStatusChanged, "support.ticket_status_changed");
event!(TicketUpdated, "support.ticket_updated");
event!(TicketAssigned, "support.ticket_assigned");
event!(MessageAdded, "support.message_added");
event!(MessageHidden, "support.message_hidden");
event!(TicketsClaimed, "support.tickets_claimed");
event!(SettingsChanged, "support.settings_changed");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        TICKET_CREATED,
        TICKET_STATUS_CHANGED,
        TICKET_UPDATED,
        TICKET_ASSIGNED,
        MESSAGE_ADDED,
        MESSAGE_HIDDEN,
        TICKETS_CLAIMED,
        SETTINGS_CHANGED,
    ]
}

/// Chủ thể (subject) của event về phiếu `id`.
pub(crate) fn ticket_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("support.ticket:{id}"))
}
