//! Thông báo của module `automations`, gửi qua `dyn Notifier` của module `notifications` (đăng ký
//! TRƯỚC) — từ job, không bao giờ trong transaction ghi. Không có `notifications` thì bước `notify`
//! được bỏ qua (ghi `skipped` vào nhật ký).

use autoxide_kernel::error::Error;
use autoxide_notifications::api::{
    Category, Channel, NotificationKind, NotificationTemplate, Notifier,
};

/// Bước `notify` của luật (và cảnh báo số liệu): câu do người tạo luật viết.
pub(crate) const ALERT: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::Email],
    NotificationTemplate::new("automations.alert", "{title}", "{body}", "{title}"),
);

pub(crate) fn declare(notifier: &dyn Notifier) -> Result<(), Error> {
    notifier.declare(ALERT)
}
