//! Bộ giả cho test: bộ gửi thông báo đếm tin (sổ cái giả dùng chung: `autoxide_ledger::api::testing`).

use std::sync::Mutex;

use autoxide_kernel::error::Error;

/// Bộ gửi thông báo giả: nhớ mọi lời gọi `notify` (kể cả trùng khoá) để đếm số lần gửi.
#[derive(Default)]
pub(crate) struct FakeNotifier {
    pub sent: Mutex<Vec<autoxide_notifications::api::Notification>>,
}

#[async_trait::async_trait]
impl autoxide_notifications::api::Notifier for FakeNotifier {
    fn declare(&self, _: autoxide_notifications::api::NotificationKind) -> Result<(), Error> {
        Ok(())
    }

    async fn notify(
        &self,
        _: autoxide_kernel::tenancy::TenantId,
        notification: autoxide_notifications::api::Notification,
    ) -> Result<autoxide_notifications::api::Outcome, Error> {
        self.sent.lock().unwrap().push(notification);
        Ok(autoxide_notifications::api::Outcome::default())
    }
}
