//! Permission của module `newsletter`. Người đăng ký là dữ liệu cá nhân: đọc cần
//! `newsletter.read`. Gửi mail tách riêng thành `newsletter.send`, nên agent soạn được chiến dịch
//! nháp mà không gửi được nếu không có quyền này. Xuất / xoá theo email (quyền của chủ dữ liệu)
//! là `newsletter.export`.

use autoxide_kernel::access::Permission;

pub(crate) const NEWSLETTER_READ: Permission = Permission::new(
    "newsletter.read",
    "Read lists, subscribers (personal data), campaigns with their progress, and suppressions.",
);
pub(crate) const NEWSLETTER_MANAGE: Permission = Permission::new(
    "newsletter.manage",
    "Create and change lists, import and change subscribers, write draft campaigns, manage \
     suppressions and report bounces.",
);
pub(crate) const NEWSLETTER_SEND: Permission = Permission::new(
    "newsletter.send",
    "Send, schedule, pause, resume and cancel campaigns, and send test mails.",
);
pub(crate) const NEWSLETTER_EXPORT: Permission = Permission::new(
    "newsletter.export",
    "Export or erase everything the newsletter holds about an email address.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![NEWSLETTER_READ, NEWSLETTER_MANAGE, NEWSLETTER_SEND, NEWSLETTER_EXPORT]
}
