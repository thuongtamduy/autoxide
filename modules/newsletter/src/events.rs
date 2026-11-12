//! Event module `newsletter` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module
//! (audit ghi "trước / sau"). Event ghi CÙNG transaction với thay đổi.
//!
//! Event **không mang địa chỉ email** hay tên người đăng ký: module `webhooks` chuyển mọi event ra
//! ngoài, nên payload chỉ có ID, `key` của danh sách, trạng thái và số đếm. Hệ thống ngoài lấy
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
    (ListCreated, "newsletter.list_created", "A list was created."),
    (ListUpdated, "newsletter.list_updated", "A list changed."),
    (ListDeleted, "newsletter.list_deleted", "A list was deleted, with its subscriptions."),
    (SubscriptionRequested, "newsletter.subscription_requested", "Someone asked to join a list that needs confirming; a confirmation mail may be on its way. No address in the payload."),
    (SubscriptionConfirmed, "newsletter.subscription_confirmed", "A subscription is confirmed: by the confirmation link, at once on a list without double opt-in, by an import or from the preferences page."),
    (SubscriptionUnsubscribed, "newsletter.subscription_unsubscribed", "A subscriber left a list (one-click link, preferences page or an admin)."),
    (SubscriberUpdated, "newsletter.subscriber_updated", "An admin changed a subscriber (name, language, blocked or not, attributes)."),
    (SubscribersImported, "newsletter.subscribers_imported", "Subscribers were imported into lists; counts only."),
    (SubscriberErased, "newsletter.subscriber_erased", "Everything about a subscriber was deleted (by an admin or a data subject request); the address is not in the payload."),
    (SubscriptionsExpired, "newsletter.subscriptions_expired", "The daily job deleted subscriptions left unconfirmed too long."),
    (SuppressionAdded, "newsletter.suppression_added", "An address was put on the suppression list (hard bounce, complaint, soft bounces or by hand); it gets no campaign mail. No address in the payload."),
    (SuppressionRemoved, "newsletter.suppression_removed", "An address was taken off the suppression list."),
    (CampaignCreated, "newsletter.campaign_created", "A draft campaign was created (by hand or for a new post)."),
    (CampaignUpdated, "newsletter.campaign_updated", "A draft or scheduled campaign changed."),
    (CampaignDeleted, "newsletter.campaign_deleted", "A campaign that never sent was deleted."),
    (CampaignScheduled, "newsletter.campaign_scheduled", "A campaign was scheduled to start sending at `scheduled_at`."),
    (CampaignStarted, "newsletter.campaign_started", "A campaign started sending."),
    (CampaignPaused, "newsletter.campaign_paused", "Sending was paused."),
    (CampaignResumed, "newsletter.campaign_resumed", "Sending resumed with the recipients left."),
    (CampaignCancelled, "newsletter.campaign_cancelled", "A campaign was cancelled; mails not sent yet are skipped."),
    (CampaignSent, "newsletter.campaign_sent", "Every recipient of a campaign was handled; counts by delivery status."),
}

/// Chủ thể (subject) của event về danh sách `id`.
pub(crate) fn list_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("newsletter.list:{id}"))
}

/// Chủ thể của event về người đăng ký `id`.
pub(crate) fn subscriber_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("newsletter.subscriber:{id}"))
}

/// Chủ thể của event về chiến dịch `id`.
pub(crate) fn campaign_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("newsletter.campaign:{id}"))
}

/// Chủ thể của event về một dòng của danh sách chặn.
pub(crate) fn suppression_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("newsletter.suppression:{id}"))
}
