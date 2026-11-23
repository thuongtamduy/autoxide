//! Báo tin chưa đọc cho người không online (gom lô): mỗi tin mới hẹn một job
//! `conversations.notify_unread` sau `AUTOXIDE_CONVERSATIONS_NOTIFY_AFTER_MINUTES` phút. Khung bắt
//! đầu từ tin chưa đọc đầu tiên (cột `notify_<phía>_from` của cuộc, đổi có điều kiện cùng transaction
//! với tin): chỉ tin mở khung mới hẹn job, mọi tin trong khung chung một job, một thông báo; đọc hết
//! thì khung đóng. Tới giờ mà phía nhận đã đọc tới tin đó (con trỏ đọc) thì không báo. Thông báo là tin giao
//! dịch, gửi qua `dyn Notifier` của `notifications`; khoá chống trùng của thông báo là khoá của job,
//! nên job chạy lại không báo hai lần. Không cài `notifications` thì không gửi gì.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::job::{Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{read, thread};
use crate::service::ConversationsService;

pub(crate) const NOTIFY_UNREAD: &str = "conversations.notify_unread";

pub(crate) const CUSTOMER_UNREAD: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::InApp, Channel::MobilePush],
    NotificationTemplate::new(
        "conversations.shop_replied",
        "New message from the shop",
        "The shop answered you in the chat. Open the conversation to read it and reply.",
        "New message from the shop",
    ),
);

pub(crate) const STAFF_UNREAD: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::Email, Channel::MobilePush],
    NotificationTemplate::new(
        "conversations.customer_wrote",
        "A customer is waiting in the chat",
        "A customer wrote in a conversation assigned to you and nobody has read it yet.",
        "A customer is waiting in the chat",
    ),
);

pub(crate) const CALLBACK_EXPIRED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::InApp, Channel::MobilePush],
    NotificationTemplate::new(
        "conversations.callback_expired",
        "We could not call you back",
        "The shop could not call you back in the time you chose. Open the chat to choose another time.",
        "Choose another time for your call-back",
    ),
);

/// Khai các loại thông báo của `conversations`.
pub(crate) fn declare(notifier: &dyn Notifier) -> Result<(), Error> {
    for kind in [CUSTOMER_UNREAD, STAFF_UNREAD, CALLBACK_EXPIRED] {
        notifier.declare(kind)?;
    }
    Ok(())
}

/// Job báo tin chưa đọc.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct NotifyJob {
    pub thread_id: Uuid,
    /// `customer` / `staff`: phía cần báo.
    pub side: String,
    /// Tin đầu tiên của khung.
    pub message_id: Uuid,
}

impl Job for NotifyJob {
    const KIND: &'static str = NOTIFY_UNREAD;
}

pub(crate) struct Unread(pub Arc<ConversationsService>);

#[async_trait::async_trait]
impl JobHandler for Unread {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else {
            return Ok(());
        };
        let payload: NotifyJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.notify_unread(tenant, &payload, job.id).await.map(|_| ())
    }
}

impl ConversationsService {
    /// Báo phía `side` của cuộc trò chuyện nếu tin `message_id` vẫn chưa đọc; trả có báo không.
    pub(crate) async fn notify_unread(
        &self,
        tenant: &TenantContext,
        job: &NotifyJob,
        key: Uuid,
    ) -> Result<bool, Error> {
        let Some(notifier) = &self.parts.notifier else {
            return Ok(false);
        };
        let Some(row) = thread::Entity::find_by_id(job.thread_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
        else {
            return Ok(false);
        };
        let (recipient, unread, kind, link) = match job.side.as_str() {
            "customer" => (
                row.customer_user_id,
                row.unread_customer,
                CUSTOMER_UNREAD,
                format!("/conversations/me/threads/{}", row.id),
            ),
            _ => (
                row.assignee_user_id,
                row.unread_staff,
                STAFF_UNREAD,
                format!("/conversations/threads/{}", row.id),
            ),
        };
        let Some(recipient) = recipient else {
            return Ok(false);
        };
        if unread == 0 {
            return Ok(false);
        }
        // Đã đọc tới (hay qua) tin của khung: không báo.
        let seen = read::Entity::find()
            .filter(read::Column::ThreadId.eq(row.id))
            .filter(read::Column::UserId.eq(recipient))
            .filter(read::Column::LastReadMessageId.gte(job.message_id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        if seen.is_some() {
            return Ok(false);
        }
        let mut notification = Notification::new(
            kind.key(),
            Recipient::User(recipient),
            json!({"unread": unread}),
            format!("conversations.unread:{key}"),
        )
        .link(link);
        if job.side == "customer"
            && let Some(locale) = &row.locale
        {
            notification = notification.locale(locale.clone());
        }
        notifier.notify(tenant.id(), notification).await?;
        Ok(true)
    }
}
