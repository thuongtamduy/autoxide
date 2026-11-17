//! Permission của module `notifications`. Hộp thư, tuỳ chọn nhận và đăng ký đẩy của chính mình chỉ
//! cần đăng nhập.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const SEND: Permission = Permission::new(
    "notifications.send",
    "Send security and transactional notifications of a declared kind to people of the tenant.",
)
.with_risk(Risk::Write);

pub(crate) const SEND_MARKETING: Permission = Permission::new(
    "notifications.send_marketing",
    "Send advertising notifications (only to people who consented, inside the allowed hours); \
     needs `notifications.send` too.",
)
.with_risk(Risk::Write);

pub(crate) const MESSAGES_READ: Permission = Permission::new(
    "notifications.messages.read",
    "See the send log of notifications (addresses of recipients included).",
)
.with_risk(Risk::SensitiveData);

pub(crate) const TEMPLATES_MANAGE: Permission = Permission::new(
    "notifications.templates.manage",
    "Replace the default texts of notification kinds for the tenant.",
)
.with_risk(Risk::Write);

pub(crate) const PLATFORM_NOTICES_SEND: Permission = Permission::new(
    "notifications.platform_notices.send",
    "Platform staff: send a notice to the admins of every tenant (in-app and email).",
)
.with_risk(Risk::Write);

pub(crate) fn all() -> Vec<Permission> {
    vec![SEND, SEND_MARKETING, MESSAGES_READ, TEMPLATES_MANAGE, PLATFORM_NOTICES_SEND]
}
