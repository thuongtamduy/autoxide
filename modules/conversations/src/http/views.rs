//! Hình dạng JSON trả ra. Nhân viên thấy bản nháp AI và số tin người mua chưa được đọc; người mua thấy
//! số tin của cửa hàng mình chưa đọc.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::read;
use crate::service::{Message, Thread, media_of};

/// A conversation between a customer and a unit (shop or marketplace store).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = Conversation)]
pub(crate) struct ThreadResponse {
    pub id: Uuid,
    /// The organization unit that answers.
    pub org_id: Uuid,
    /// The customer; empty after the customer deleted their account.
    pub customer_user_id: Option<Uuid>,
    /// The record asked about, `<kind>:<id>` (e.g. `catalog.product:<id>`).
    pub subject: Option<String>,
    /// `open` or `closed`. A closed conversation takes no more messages.
    #[schema(example = "open")]
    pub status: String,
    pub assignee_user_id: Option<Uuid>,
    /// Messages from the other side you have not read: for staff, the customer's; for the
    /// customer, the shop's.
    pub unread: i32,
    /// The customer asked for a person, or an automatic answer handed the conversation over.
    pub needs_agent: bool,
    /// Automatic answers are stopped in this conversation (the customer asked for a person).
    pub auto_reply_paused: bool,
    /// The support ticket this conversation became.
    pub ticket_id: Option<Uuid>,
    /// AI draft for an agent to edit and send (staff only); never sent by itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_reply: Option<String>,
    pub last_message_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ThreadResponse {
    /// `staff`: the view of an agent.
    pub(crate) fn new(thread: Thread, staff: bool) -> Self {
        ThreadResponse {
            id: thread.id,
            org_id: thread.org_id,
            customer_user_id: thread.customer_user_id,
            subject: thread.subject,
            status: thread.status,
            assignee_user_id: thread.assignee_user_id,
            unread: if staff { thread.unread_staff } else { thread.unread_customer },
            needs_agent: thread.needs_agent,
            auto_reply_paused: thread.auto_reply_paused,
            ticket_id: thread.ticket_id,
            draft_reply: if staff { thread.draft_reply } else { None },
            last_message_at: thread.last_message_at,
            closed_at: thread.closed_at,
            created_at: thread.created_at,
            updated_at: thread.updated_at,
        }
    }
}

/// A chat message. Messages cannot be edited; a recalled message keeps its place without text.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationMessage)]
pub(crate) struct MessageResponse {
    pub id: Uuid,
    pub thread_id: Uuid,
    /// `customer`, `agent` or `system` (automatic answers and call summaries).
    #[schema(example = "customer")]
    pub author_kind: String,
    pub author_user_id: Option<Uuid>,
    /// Plain text; empty when recalled. Show it as text, never as HTML.
    pub body: String,
    /// Images from the media library.
    pub media_ids: Vec<Uuid>,
    /// The id the sending app gave this message.
    pub client_id: String,
    /// The call this system message sums up.
    pub call_id: Option<Uuid>,
    /// Sent automatically from an approved answer: show the label "Automatic answer" and a
    /// "Talk to a person" button.
    pub auto_reply: bool,
    /// The author recalled the message.
    pub recalled: bool,
    pub created_at: DateTime<Utc>,
}

impl From<Message> for MessageResponse {
    fn from(message: Message) -> Self {
        let media_ids = media_of(&message);
        MessageResponse {
            id: message.id,
            thread_id: message.thread_id,
            author_kind: message.author_kind,
            author_user_id: message.author_user_id,
            body: message.body,
            media_ids,
            client_id: message.client_id,
            call_id: message.call_id,
            auto_reply: message.auto_reply,
            recalled: message.recalled_at.is_some(),
            created_at: message.created_at,
        }
    }
}

/// How far one person has read.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationReadCursor)]
pub(crate) struct ReadResponse {
    pub user_id: Uuid,
    /// The last message this person has read.
    pub last_read_message_id: Uuid,
    pub read_at: DateTime<Utc>,
}

impl From<read::Model> for ReadResponse {
    fn from(row: read::Model) -> Self {
        ReadResponse {
            user_id: row.user_id,
            last_read_message_id: row.last_read_message_id,
            read_at: row.read_at,
        }
    }
}

/// A conversation with how far each person has read it.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationDetail)]
pub(crate) struct ThreadDetail {
    #[serde(flatten)]
    pub thread: ThreadResponse,
    pub reads: Vec<ReadResponse>,
}

/// A conversation just started (or continued) with the message just sent.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = StartedConversation)]
pub(crate) struct StartedResponse {
    pub thread: ThreadResponse,
    pub message: MessageResponse,
}

/// A report of abuse.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationReport)]
pub(crate) struct ReportResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub thread_id: Uuid,
    pub message_id: Option<Uuid>,
    /// `customer` or `agent`.
    pub reporter_kind: String,
    pub reason: String,
    /// `open` or `resolved`.
    pub status: String,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<crate::abuse::Report> for ReportResponse {
    fn from(row: crate::abuse::Report) -> Self {
        ReportResponse {
            id: row.id,
            org_id: row.org_id,
            thread_id: row.thread_id,
            message_id: row.message_id,
            reporter_kind: row.reporter_kind,
            reason: row.reason,
            status: row.status,
            resolved_at: row.resolved_at,
            created_at: row.created_at,
        }
    }
}

/// The support ticket a conversation became.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationEscalation)]
pub(crate) struct EscalatedResponse {
    pub thread: ThreadResponse,
    pub ticket_id: Uuid,
    /// Ticket number shown to the customer.
    pub ticket_number: String,
}

/// Whether the customer of a conversation is blocked.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationBlock)]
pub(crate) struct BlockResponse {
    pub blocked: bool,
}
