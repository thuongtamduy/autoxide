//! Hình dạng JSON trả ra. Nhân viên thấy liên lạc đầy đủ; người mua và khách vãng lai thấy email /
//! số điện thoại đã che (D-202) và không thấy ghi chú nội bộ.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::service::{Message, Ticket, body_html, mask_email, mask_phone};
use crate::status::{AuthorKind, Channel, MessageKind, Priority, Resolution, Status};

/// A support ticket.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportTicket)]
pub(crate) struct TicketResponse {
    pub id: Uuid,
    /// Number shown to the customer.
    #[schema(example = "000123")]
    pub number: String,
    /// The organization unit (shop, marketplace store) the ticket belongs to.
    pub org_id: Uuid,
    pub subject: String,
    pub category: String,
    pub priority: Priority,
    pub status: Status,
    /// How the ticket ended; set once solved or closed.
    pub resolution: Option<Resolution>,
    pub channel: Channel,
    pub requester: RequesterView,
    pub assignee_user_id: Option<Uuid>,
    pub team_id: Option<Uuid>,
    /// Answer the customer before this (business hours of the unit).
    pub first_response_due_at: Option<DateTime<Utc>>,
    /// Solve the ticket before this; the clock stops while waiting for the customer.
    pub resolve_due_at: Option<DateTime<Utc>>,
    pub first_responded_at: Option<DateTime<Utc>>,
    /// A response-time target was missed.
    pub breached: bool,
    /// The consumer complaint this ticket became (module `compliance`).
    pub case_id: Option<Uuid>,
    /// The earlier ticket this one follows up.
    pub follow_up_of: Option<Uuid>,
    /// Handed over to the marketplace operator.
    pub escalated_to_platform: bool,
    /// Customer rating after the ticket was solved, 1 to 5.
    pub satisfaction: Option<i16>,
    pub last_message_at: DateTime<Utc>,
    pub solved_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Who asked. Customers and guests see their own email or phone number masked.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportRequester)]
pub(crate) struct RequesterView {
    /// The customer account, when the ticket belongs to one.
    pub user_id: Option<Uuid>,
    pub name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    /// The guest proved the email or phone number (accounts always count as verified).
    pub verified: bool,
}

impl TicketResponse {
    /// `full`: show the contact unmasked (staff).
    pub(crate) fn new(ticket: Ticket, full: bool) -> Self {
        let email =
            ticket.requester_email.map(|email| if full { email } else { mask_email(&email) });
        let phone =
            ticket.requester_phone.map(|phone| if full { phone } else { mask_phone(&phone) });
        TicketResponse {
            id: ticket.id,
            number: ticket.number,
            org_id: ticket.org_id,
            subject: ticket.subject,
            category: ticket.category,
            priority: Priority::parse(&ticket.priority).unwrap_or(Priority::Normal),
            status: Status::parse(&ticket.status).unwrap_or(Status::Open),
            resolution: ticket.resolution.as_deref().and_then(Resolution::parse),
            channel: Channel::parse(&ticket.channel).unwrap_or(Channel::Web),
            requester: RequesterView {
                user_id: ticket.requester_user_id,
                name: ticket.requester_name,
                email,
                phone,
                verified: ticket.verified_at.is_some(),
            },
            assignee_user_id: ticket.assignee_user_id,
            team_id: ticket.team_id,
            first_response_due_at: ticket.first_response_due_at,
            resolve_due_at: ticket.resolve_due_at,
            first_responded_at: ticket.first_responded_at,
            breached: ticket.breached,
            case_id: ticket.case_id,
            follow_up_of: ticket.follow_up_of,
            escalated_to_platform: ticket.escalated_to_platform,
            satisfaction: ticket.satisfaction,
            last_message_at: ticket.last_message_at,
            solved_at: ticket.solved_at,
            closed_at: ticket.closed_at,
            created_at: ticket.created_at,
            updated_at: ticket.updated_at,
        }
    }
}

/// One message of a ticket. Messages are never edited; a hidden message keeps its place with
/// `hidden: true` (staff still see its text and the reason).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportMessage)]
pub(crate) struct MessageResponse {
    pub id: Uuid,
    pub kind: MessageKind,
    pub author_kind: AuthorKind,
    /// The account that wrote it (staff or customer); empty for guests and the server.
    pub author_user_id: Option<Uuid>,
    /// Plain text as written. For `system` messages: a line key such as `status:solved`.
    pub body: String,
    /// The same text as safe HTML (escaped, line breaks as `<br>`).
    pub body_html: String,
    /// Attached media items (module `media`).
    pub media_ids: Vec<Uuid>,
    /// Colleagues mentioned in an internal note.
    pub mentions: Vec<Uuid>,
    pub hidden: bool,
    /// Why it was hidden (staff only).
    pub hidden_reason: Option<String>,
    /// Written from an AI suggestion (label it).
    pub ai_assisted: bool,
    /// Sent automatically from an approved answer (label it "Automatic answer").
    pub auto_reply: bool,
    pub created_at: DateTime<Utc>,
}

impl From<Message> for MessageResponse {
    fn from(message: Message) -> Self {
        MessageResponse {
            id: message.id,
            kind: MessageKind::parse(&message.kind).unwrap_or(MessageKind::Public),
            author_kind: AuthorKind::parse(&message.author_kind).unwrap_or(AuthorKind::System),
            author_user_id: message.author_user_id,
            body_html: body_html(&message.body),
            body: message.body,
            media_ids: serde_json::from_str(&message.media_ids).unwrap_or_default(),
            mentions: serde_json::from_str(&message.mentions).unwrap_or_default(),
            hidden: message.hidden_at.is_some(),
            hidden_reason: message.hidden_reason,
            ai_assisted: message.ai_assisted,
            auto_reply: message.auto_reply,
            created_at: message.created_at,
        }
    }
}

/// A ticket with its messages, oldest first.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportTicketDetail)]
pub(crate) struct TicketDetail {
    #[serde(flatten)]
    pub ticket: TicketResponse,
    pub messages: Vec<MessageResponse>,
    /// Linked records, as `<module>.<type>:<id>` (staff only; empty for customers).
    pub links: Vec<String>,
}

impl TicketDetail {
    pub(crate) fn new(ticket: Ticket, messages: Vec<Message>, full: bool) -> Self {
        TicketDetail {
            ticket: TicketResponse::new(ticket, full),
            messages: messages.into_iter().map(MessageResponse::from).collect(),
            links: Vec::new(),
        }
    }
}

/// A ticket just opened. Guests get `access_token` once: keep it to see and answer the ticket.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportOpenedTicket)]
pub(crate) struct OpenedTicket {
    #[serde(flatten)]
    pub ticket: TicketResponse,
    /// Guests only, shown only here: send it as `token` to the `/public/tickets/{id}` endpoints.
    pub access_token: Option<String>,
}
