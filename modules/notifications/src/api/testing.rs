//! Bộ gửi thông báo giả dùng chung cho test của module khác (feature `testing`, chỉ khai ở
//! `[dev-dependencies]`; quy ước D-20261122-module-test-fakes).
//!
//! Giống `notifications` thật ở chỗ module khác dựa vào: cùng `dedupe_key` thì không tạo tin thứ hai.
//! Không gửi gì, không kiểm loại đã khai: chỉ nhớ để test đọc lại.
//!
//! ```ignore
//! let notifier = Arc::new(FakeNotifier::new());
//! let app = TestApp::builder(test).provide::<dyn Notifier>(notifier.clone()).module(my_module())…;
//! assert_eq!(notifier.sent()[0].kind, "orders.order_shipped");
//! ```

use std::sync::{Mutex, MutexGuard, PoisonError};

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantId;

use super::{Notification, NotificationKind, Notifier, Outcome};

#[derive(Default)]
struct State {
    /// Khoá các loại đã khai, theo thứ tự.
    kinds: Vec<String>,
    /// Tin đã tạo (mỗi `dedupe_key` một tin).
    sent: Vec<Notification>,
    /// Mọi lời gọi `notify`, kể cả trùng khoá.
    calls: Vec<Notification>,
}

/// Bộ gửi thông báo giả, giữ mọi thứ trong bộ nhớ. Công bố vào app bằng
/// `TestApp::builder(…).provide::<dyn Notifier>(notifier.clone())`.
#[derive(Default)]
pub struct FakeNotifier {
    state: Mutex<State>,
}

// Khoá `Mutex` mà không `unwrap`: test khác panic giữa chừng thì vẫn đọc tiếp dữ liệu bên trong.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeNotifier {
    pub fn new() -> Self {
        FakeNotifier::default()
    }

    /// Tin đã tạo, theo thứ tự: lần gọi lại cùng `dedupe_key` không thêm tin, như `notifications`.
    pub fn sent(&self) -> Vec<Notification> {
        lock(&self.state).sent.clone()
    }

    /// Tin đã tạo thuộc loại `kind`.
    pub fn sent_of(&self, kind: &str) -> Vec<Notification> {
        lock(&self.state).sent.iter().filter(|sent| sent.kind == kind).cloned().collect()
    }

    /// Mọi lời gọi `notify`, kể cả trùng `dedupe_key` (đếm số lần module gọi).
    pub fn calls(&self) -> Vec<Notification> {
        lock(&self.state).calls.clone()
    }

    /// Khoá các loại module đã khai (`declare`), theo thứ tự.
    pub fn kinds(&self) -> Vec<String> {
        lock(&self.state).kinds.clone()
    }

    /// Quên mọi tin đã nhận (giữ các loại đã khai).
    pub fn clear(&self) {
        let mut state = lock(&self.state);
        state.sent.clear();
        state.calls.clear();
    }
}

#[async_trait::async_trait]
impl Notifier for FakeNotifier {
    fn declare(&self, kind: NotificationKind) -> Result<(), Error> {
        lock(&self.state).kinds.push(kind.key().to_string());
        Ok(())
    }

    async fn notify(&self, _: TenantId, notification: Notification) -> Result<Outcome, Error> {
        let mut state = lock(&self.state);
        state.calls.push(notification.clone());
        if !state.sent.iter().any(|old| old.dedupe_key == notification.dedupe_key) {
            state.sent.push(notification);
        }
        Ok(Outcome::default())
    }
}
