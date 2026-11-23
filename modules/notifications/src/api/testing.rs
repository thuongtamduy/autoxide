//! Shared fake notification sender for other modules' tests (feature `testing`, declared only in
//! `[dev-dependencies]`; convention D-20261122-module-test-fakes).
//!
//! Behaves like the real `notifications` in the way other modules rely on: the same `dedupe_key` does not
//! create a second message. It sends nothing and does not validate declared kinds: it only remembers, so
//! tests can read back.
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
    /// Keys of the declared kinds, in order.
    kinds: Vec<String>,
    /// Messages created (one per `dedupe_key`).
    sent: Vec<Notification>,
    /// Every `notify` call, including duplicate keys.
    calls: Vec<Notification>,
}

/// Fake notification sender, keeping everything in memory. Publish it into the app with
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

    /// Messages created, in order: calling again with the same `dedupe_key` adds no message, as in `notifications`.
    pub fn sent(&self) -> Vec<Notification> {
        lock(&self.state).sent.clone()
    }

    /// Messages created of kind `kind`.
    pub fn sent_of(&self, kind: &str) -> Vec<Notification> {
        lock(&self.state).sent.iter().filter(|sent| sent.kind == kind).cloned().collect()
    }

    /// Every `notify` call, including duplicate `dedupe_key` (counts how many times the module called).
    pub fn calls(&self) -> Vec<Notification> {
        lock(&self.state).calls.clone()
    }

    /// Keys of the kinds the module declared (`declare`), in order.
    pub fn kinds(&self) -> Vec<String> {
        lock(&self.state).kinds.clone()
    }

    /// Forgets every message received (keeps the declared kinds).
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
