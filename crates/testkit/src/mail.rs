//! Fake mail sender for tests (like `FakeTextModel`): keeps mail in memory instead of sending it.
//!
//! Behaves like the real `mail` module in the way other modules rely on: once mail with a given `dedupe_key`
//! has been accepted, later mail with the same key is ignored. `reject_reply_to` simulates a mail server
//! rejecting the `reply_to` address (`validation_failed`, field `reply_to`).
//!
//! ```ignore
//! let mailer = Arc::new(FakeMailer::new());
//! let app = TestApp::builder(test).provide::<dyn Mailer>(mailer.clone())…;
//! assert_eq!(mailer.to("ada@example.com").len(), 1);
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mail::{Mail, Mailer};
use autoxide_kernel::tenancy::TenantContext;

#[derive(Default)]
struct State {
    /// Mail accepted (mail with a duplicate `dedupe_key` is not added).
    sent: Vec<Mail>,
    /// Number of `send` calls, including mail dropped as a duplicate key and mail rejected.
    calls: usize,
}

/// Fake mail sender. Publish it into the app with `TestApp::builder(…).provide::<dyn Mailer>(mailer.clone())`.
#[derive(Default)]
pub struct FakeMailer {
    state: Mutex<State>,
    reject_reply_to: AtomicBool,
}

// Khoá `Mutex` mà không `unwrap`: test khác panic giữa chừng thì vẫn đọc tiếp dữ liệu bên trong.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeMailer {
    pub fn new() -> Self {
        FakeMailer::default()
    }

    /// Mail accepted, in order.
    pub fn sent(&self) -> Vec<Mail> {
        lock(&self.state).sent.clone()
    }

    /// Mail sent to exactly the address `address`.
    pub fn to(&self, address: &str) -> Vec<Mail> {
        lock(&self.state).sent.iter().filter(|mail| mail.to == address).cloned().collect()
    }

    /// The last mail sent, if any.
    pub fn last(&self) -> Option<Mail> {
        lock(&self.state).sent.last().cloned()
    }

    /// Number of `send` calls (including mail dropped as a duplicate key and mail rejected).
    pub fn calls(&self) -> usize {
        lock(&self.state).calls
    }

    /// Forgets all mail received.
    pub fn clear(&self) {
        let mut state = lock(&self.state);
        state.sent.clear();
        state.calls = 0;
    }

    /// When enabled, every mail with a `reply_to` is rejected, as a mail server would reject that address.
    pub fn reject_reply_to(&self, reject: bool) {
        self.reject_reply_to.store(reject, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl Mailer for FakeMailer {
    async fn send(&self, _: &TenantContext, mail: Mail) -> Result<(), Error> {
        let mut state = lock(&self.state);
        state.calls += 1;
        if mail.reply_to.is_some() && self.reject_reply_to.load(Ordering::SeqCst) {
            return Err(Error::validation(vec![FieldError::new(
                "reply_to",
                "email",
                "must be a mail address, like `ada@example.com`",
            )]));
        }
        let duplicate = mail.dedupe_key.is_some()
            && state.sent.iter().any(|sent| sent.dedupe_key == mail.dedupe_key);
        if !duplicate {
            state.sent.push(mail);
        }
        Ok(())
    }
}
