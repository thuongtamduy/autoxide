//! Thông báo của nền tảng tới quản trị viên khách về thuê bao (bước 35, D-348): quá hạn thanh toán,
//! tạm khoá, sắp chạm hạn mức. Gửi qua `dyn Notifier` của module `notifications` (đăng ký TRƯỚC
//! `plans`) — từ handler event và job, không bao giờ trong transaction ghi. Không có `notifications`
//! thì không gửi gì (mail nhắc thanh toán vẫn đi như cũ).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::tenancy::TenantId;
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::service::PlansService;

pub(crate) const PAST_DUE: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::Email],
    NotificationTemplate::new(
        "plans.subscription_past_due",
        "Your subscription payment is overdue",
        "The payment of your subscription is overdue. Pay it during the grace period to keep full \
         access; after that the data can only be read.",
        "Subscription payment overdue",
    ),
);

pub(crate) const SUSPENDED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::Email],
    NotificationTemplate::new(
        "plans.subscription_suspended",
        "Your subscription is suspended",
        "Your subscription is suspended because a payment is overdue: the data can only be read \
         until it is paid.",
        "Subscription suspended",
    ),
);

pub(crate) const NEAR_LIMIT: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::Email],
    NotificationTemplate::new(
        "plans.usage_near_limit",
        "You are close to a limit of your plan",
        "You have used at least 80% of a limit of your plan. Move to a larger plan or add an add-on \
         before you reach it.",
        "Close to a plan limit",
    ),
);

/// Khai các loại thông báo của `plans`.
pub(crate) fn declare(notifier: &dyn Notifier) -> Result<(), Error> {
    for kind in [PAST_DUE, SUSPENDED, NEAR_LIMIT] {
        notifier.declare(kind)?;
    }
    Ok(())
}

/// Nghe `plans.subscription_status_changed`: sang `past_due` hay `suspended` thì báo quản trị viên
/// khách. Khoá chống trùng là ID event: event giao lại không gửi lần hai.
pub(crate) struct StatusNotices(pub Arc<PlansService>);

#[async_trait::async_trait]
impl EventHandler for StatusNotices {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(notifier) = &self.0.notifier else {
            return Ok(());
        };
        let kind = match event.payload.get("to").and_then(Value::as_str) {
            Some("past_due") => PAST_DUE,
            Some("suspended") => SUSPENDED,
            _ => return Ok(()),
        };
        let Some(tenant) = event
            .payload
            .get("tenant_id")
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        let notification = Notification::new(
            kind.key(),
            Recipient::TenantAdmins,
            json!({}),
            format!("plans.status:{}", event.id),
        )
        .link("/billing");
        notifier.notify(TenantId::from_uuid(tenant), notification).await.map(|_| ())
    }
}

impl PlansService {
    /// Báo quản trị viên của khách đã dùng từ 80% một hạn mức — mỗi khách tối đa một lần mỗi tháng.
    /// Chạy sau lượt đối soát hằng ngày; lỗi của một khách chỉ ghi log.
    pub(crate) async fn notify_near_limit(&self, at: Timestamp) -> Result<usize, Error> {
        let Some(notifier) = &self.notifier else {
            return Ok(0);
        };
        let month = at.format("%Y-%m").to_string();
        let mut sent = 0;
        for tenant in self.tenants_near_limit().await? {
            let notification = Notification::new(
                NEAR_LIMIT.key(),
                Recipient::TenantAdmins,
                json!({}),
                format!("plans.near_limit:{month}"),
            )
            .link("/billing");
            match notifier.notify(tenant, notification).await {
                Ok(_) => sent += 1,
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant, error = %error, "near-limit notice failed");
                }
            }
        }
        Ok(sent)
    }
}
