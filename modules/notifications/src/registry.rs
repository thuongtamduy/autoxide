//! Sổ loại thông báo: module khác khai qua `Notifier::declare` trong `Module::start` của chúng — SAU
//! khi `notifications` đã khởi động — nên sổ dùng chung (`Arc`) và khoá đọc / ghi bên trong.

use std::collections::BTreeMap;
use std::sync::RwLock;

use autoxide_kernel::error::Error;

use crate::api::{Category, Channel, NotificationKind, NotificationTemplate};

/// Loại thông báo của nền tảng gửi tới quản trị viên mọi khách (giao dịch: tắt được).
pub(crate) const PLATFORM_NOTICE: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp, Channel::Email],
    NotificationTemplate::new("notifications.platform_notice", "{title}", "{body}", "{title}"),
);

/// Job người dùng yêu cầu (bản xuất, thao tác hàng loạt…) đã xong hay hỏng (bước 51).
pub(crate) const JOB_FINISHED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp],
    NotificationTemplate::new(
        "notifications.job_finished",
        "{label}: {status}",
        "The background job you asked for ({label}) is {status}. Open the jobs list to see the result or download the file.",
        "{label}: {status}",
    ),
);

/// Ai đó nhắc bạn trong ghi chú nội bộ trên một bản ghi (bước 51 P13).
pub(crate) const MENTIONED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::InApp],
    NotificationTemplate::new(
        "notifications.mentioned",
        "You were mentioned in a note",
        "You were mentioned in an internal note on {kind} {id}.",
        "Mentioned in a note",
    ),
);

/// Thông báo bảo mật của nền tảng (không tắt được).
pub(crate) const PLATFORM_SECURITY_NOTICE: NotificationKind = NotificationKind::new(
    Category::Security,
    &[Channel::InApp, Channel::Email],
    NotificationTemplate::new(
        "notifications.platform_security_notice",
        "{title}",
        "{body}",
        "{title}",
    ),
);

#[derive(Default)]
pub(crate) struct Kinds(RwLock<BTreeMap<&'static str, NotificationKind>>);

/// `true` khi `key` có dạng `<module>.<loại>` (chữ thường, số, `_`, ngăn bằng `.`).
fn valid_key(key: &str) -> bool {
    key.len() <= 100
        && key.contains('.')
        && key.split('.').all(|part| {
            part.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

impl Kinds {
    pub(crate) fn declare(&self, kind: NotificationKind) -> Result<(), Error> {
        let key = kind.key();
        if !valid_key(key) {
            return Err(Error::internal(format!(
                "notification kind `{key}` must be `<module>.<name>` in snake_case"
            )));
        }
        if kind.channels.is_empty() {
            return Err(Error::internal(format!("notification kind `{key}` has no channel")));
        }
        if kind.invitation && kind.category != Category::Marketing {
            return Err(Error::internal(format!(
                "notification kind `{key}` is an invitation, so it must be in the marketing category"
            )));
        }
        // Khoá hỏng (luồng khác panic khi đang giữ) vẫn dùng được dữ liệu bên trong.
        let mut kinds = self.0.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        if kinds.contains_key(key) {
            return Err(Error::internal(format!("notification kind `{key}` is declared twice")));
        }
        kinds.insert(key, kind);
        Ok(())
    }

    pub(crate) fn get(&self, key: &str) -> Option<NotificationKind> {
        self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner).get(key).copied()
    }

    /// Mọi loại, theo thứ tự khoá.
    pub(crate) fn all(&self) -> Vec<NotificationKind> {
        self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner).values().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn khai_trung_hay_sai_dang_la_loi() {
        let kinds = Kinds::default();
        kinds.declare(PLATFORM_NOTICE).unwrap();
        assert!(kinds.declare(PLATFORM_NOTICE).is_err());
        let bad = NotificationKind::new(
            Category::Transactional,
            &[Channel::Email],
            NotificationTemplate::new("NoModule", "t", "b", "s"),
        );
        assert!(kinds.declare(bad).is_err());
        let invitation = NotificationKind::new(
            Category::Transactional,
            &[Channel::Sms],
            NotificationTemplate::new("x.invite", "t", "b", "s"),
        )
        .invitation();
        assert!(kinds.declare(invitation).is_err());
        assert_eq!(kinds.all().len(), 1);
        assert!(kinds.get("notifications.platform_notice").is_some());
    }
}
