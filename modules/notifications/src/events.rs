//! Event module `notifications` phát. Không phát event cho từng tin (audit phình; nhật ký gửi đã là
//! `notifications_messages`). Payload chỉ ID — không email, số điện thoại, endpoint hay token.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

macro_rules! events {
    ($($constant:ident, $kind:ident, $name:literal, $description:literal;)*) => {
        $(
            pub(crate) const $constant: EventInfo = EventInfo::new($name, $description);

            #[derive(Debug, Serialize)]
            pub(crate) struct $kind(pub Value);

            impl Event for $kind {
                const NAME: &'static str = $name;
            }
        )*

        pub(crate) fn all() -> Vec<EventInfo> {
            vec![$($constant),*]
        }
    };
}

events! {
    PREFERENCES_UPDATED, PreferencesUpdated, "notifications.preferences_updated", "A person changed which notifications they get (`user_id`, `changes`).";
    TEMPLATE_UPDATED, TemplateUpdated, "notifications.template_updated", "The text of a notification kind was replaced for the tenant (`kind`, `channel`, `locale`).";
    TEMPLATE_DELETED, TemplateDeleted, "notifications.template_deleted", "A replaced text was removed; the default text applies again (`kind`, `channel`, `locale`).";
    PUSH_SUBSCRIPTION_ADDED, PushSubscriptionAdded, "notifications.push_subscription_added", "A browser or a mobile device was registered for push notifications (`id`, `user_id`, `channel`).";
    PUSH_SUBSCRIPTION_REMOVED, PushSubscriptionRemoved, "notifications.push_subscription_removed", "A browser or a mobile device stopped getting push notifications (`id`, `user_id`, `channel`, `reason`).";
    PLATFORM_NOTICE_QUEUED, PlatformNoticeQueued, "notifications.platform_notice_queued", "Platform staff queued a notice to the admins of every tenant (`notice_id`, `category`).";
}
