//! Permission của module `newsletter`. Người đăng ký là dữ liệu cá nhân: đọc cần
//! `newsletter.read`. Gửi mail tách riêng thành `newsletter.send`, nên agent soạn được chiến dịch
//! nháp mà không gửi được nếu không có quyền này. Xuất / xoá theo email (quyền của chủ dữ liệu)
//! là `newsletter.export`. Việc dẫn tới gửi mail mà không ai có `newsletter.send` duyệt (bật
//! `new_posts = send`) cũng cần `newsletter.send`. Đăng ký thay người lạ mà không qua lớp chống
//! spam cần `newsletter.subscribe` — chỉ đăng nhập thôi là chưa đủ.

use autoxide_kernel::access::{Permission, Risk};

pub(crate) const NEWSLETTER_READ: Permission = Permission::new(
    "newsletter.read",
    "Read lists, subscribers (personal data), campaigns with their progress, and suppressions.",
)
.with_risk(Risk::SensitiveData);
pub(crate) const NEWSLETTER_MANAGE: Permission = Permission::new(
    "newsletter.manage",
    "Create and change lists, import and change subscribers, write draft campaigns (changing a \
     scheduled one moves it back to draft), manage suppressions and report bounces.",
);
pub(crate) const NEWSLETTER_SEND: Permission = Permission::new(
    "newsletter.send",
    "Send, schedule, pause, resume and cancel campaigns, send test mails, and set a list to \
     send new posts automatically (`new_posts: send`).",
);
pub(crate) const NEWSLETTER_EXPORT: Permission = Permission::new(
    "newsletter.export",
    "Export or erase everything the newsletter holds about an email address.",
)
.with_risk(Risk::SensitiveData);

pub(crate) const NEWSLETTER_SUBSCRIBE: Permission = Permission::new(
    "newsletter.subscribe",
    "Add sign-ups without the anti-spam checks for anonymous visitors — for a website server's \
     own API key; still limited per account.",
);

pub(crate) fn all() -> Vec<Permission> {
    vec![
        NEWSLETTER_READ,
        NEWSLETTER_MANAGE,
        NEWSLETTER_SEND,
        NEWSLETTER_EXPORT,
        NEWSLETTER_SUBSCRIBE,
    ]
}
