//! Thông báo của phiếu, gửi qua `dyn Notifier` của module `notifications` — tin **giao dịch** (không
//! cần đồng ý quảng cáo, không kèm quảng cáo). Gửi từ handler event `support.message_added` (ngoài
//! transaction ghi); event giao lại không gửi lần hai (khoá chống trùng là ID event + người nhận).
//! Không cài `notifications` thì không gửi gì.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::tenancy::ScopeExt;
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient,
};
use sea_orm::EntityTrait;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::ticket;
use crate::service::SupportService;

pub(crate) const VERIFICATION_CODE: NotificationKind = NotificationKind::new(
    Category::Security,
    &[Channel::Email, Channel::Sms],
    NotificationTemplate::new(
        "support.verification_code",
        "Your code for ticket {number}",
        "Enter {code} to confirm your contact for support ticket {number}. The code works for 15 \
         minutes.",
        "Support ticket {number}: your code is {code}",
    ),
);

pub(crate) const TICKET_REPLIED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::InApp],
    NotificationTemplate::new(
        "support.ticket_replied",
        "New answer on ticket {number}",
        "Our team answered your support ticket {number} ({subject}). Open the ticket to read it \
         and reply.",
        "Ticket {number}: new answer",
    ),
);

pub(crate) const CUSTOMER_REPLIED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp],
    NotificationTemplate::new(
        "support.customer_replied",
        "The customer wrote on ticket {number}",
        "The customer wrote again on support ticket {number} ({subject}).",
        "Ticket {number}: customer wrote",
    ),
);

pub(crate) const MENTIONED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::Email],
    NotificationTemplate::new(
        "support.mentioned",
        "You were mentioned on ticket {number}",
        "A colleague mentioned you in an internal note on support ticket {number} ({subject}).",
        "Mentioned on ticket {number}",
    ),
);

/// Khai các loại thông báo của `support`.
pub(crate) fn declare(notifier: &dyn Notifier) -> Result<(), Error> {
    for kind in [VERIFICATION_CODE, TICKET_REPLIED, CUSTOMER_REPLIED, MENTIONED] {
        notifier.declare(kind)?;
    }
    Ok(())
}

/// Nghe `support.message_added`: nhân viên trả lời → báo người mua; người mua viết → báo người
/// được giao; ghi chú nội bộ nhắc tên → báo người được nhắc.
pub(crate) struct MessageNotices(pub Arc<SupportService>);

#[async_trait::async_trait]
impl EventHandler for MessageNotices {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(notifier) = &self.0.parts.notifier else {
            return Ok(());
        };
        let after = &event.payload["after"];
        let Some(ticket_id) = after["ticket_id"].as_str().and_then(|raw| raw.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        let tenant = &event.tenant;
        let Some(ticket) = ticket::Entity::find_by_id(ticket_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(());
        };
        let params = json!({"number": ticket.number, "subject": ticket.subject});
        let link = format!("/support/tickets/{ticket_id}");
        let mut sends: Vec<(NotificationKind, Recipient)> = Vec::new();
        match (after["kind"].as_str(), after["author_kind"].as_str()) {
            (Some("public"), Some("agent")) => {
                let recipient = match (
                    ticket.requester_user_id,
                    &ticket.requester_email,
                    &ticket.requester_phone,
                ) {
                    (Some(user), _, _) => Some(Recipient::User(user)),
                    (None, Some(email), _) => Some(Recipient::Email(email.clone())),
                    (None, None, Some(phone)) => Some(Recipient::Phone(phone.clone())),
                    _ => None,
                };
                if let Some(recipient) = recipient {
                    sends.push((TICKET_REPLIED, recipient));
                }
            }
            (Some("public"), Some("customer")) => {
                if let Some(assignee) = ticket.assignee_user_id {
                    sends.push((CUSTOMER_REPLIED, Recipient::User(assignee)));
                }
            }
            (Some("internal"), _) => {
                let author =
                    after["author_user_id"].as_str().and_then(|raw| raw.parse::<Uuid>().ok());
                for user in mentions(&after["mentions"]) {
                    if Some(user) != author {
                        sends.push((MENTIONED, Recipient::User(user)));
                    }
                }
            }
            _ => {}
        }
        for (kind, recipient) in sends {
            let key = format!("support.notice:{}:{}", event.id, recipient_key(&recipient));
            let mut notification =
                Notification::new(kind.key(), recipient, params.clone(), key).link(link.clone());
            if kind.key() == TICKET_REPLIED.key()
                && let Some(locale) = &ticket.locale
            {
                notification = notification.locale(locale.clone());
            }
            notifier.notify(tenant.id(), notification).await?;
        }
        Ok(())
    }
}

fn mentions(raw: &Value) -> Vec<Uuid> {
    raw.as_array()
        .map(|list| list.iter().filter_map(|item| item.as_str()?.parse().ok()).collect())
        .unwrap_or_default()
}

fn recipient_key(recipient: &Recipient) -> String {
    match recipient {
        Recipient::User(id) => format!("user:{id}"),
        Recipient::Email(email) => format!("email:{}", autoxide_kernel::secret::sha256(email)),
        Recipient::Phone(phone) => format!("phone:{}", autoxide_kernel::secret::sha256(phone)),
        Recipient::TenantAdmins => "admins".to_owned(),
    }
}
