//! Bên gửi mail giả cho test (như `FakeTextModel`): giữ mail trong bộ nhớ thay vì gửi.
//!
//! Giống module `mail` thật ở chỗ module khác dựa vào: mail cùng `dedupe_key` đã nhận thì mail sau
//! bị bỏ qua. `reject_reply_to` giả máy chủ mail từ chối địa chỉ `reply_to`
//! (`validation_failed`, trường `reply_to`).
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
    /// Mail đã nhận (mail trùng `dedupe_key` không thêm).
    sent: Vec<Mail>,
    /// Số lần `send` được gọi, kể cả mail trùng khoá bị bỏ và mail bị từ chối.
    calls: usize,
}

/// Bên gửi mail giả. Công bố vào app bằng `TestApp::builder(…).provide::<dyn Mailer>(mailer.clone())`.
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

    /// Mail đã nhận, theo thứ tự.
    pub fn sent(&self) -> Vec<Mail> {
        lock(&self.state).sent.clone()
    }

    /// Mail gửi tới đúng địa chỉ `address`.
    pub fn to(&self, address: &str) -> Vec<Mail> {
        lock(&self.state).sent.iter().filter(|mail| mail.to == address).cloned().collect()
    }

    /// Mail gửi sau cùng, nếu có.
    pub fn last(&self) -> Option<Mail> {
        lock(&self.state).sent.last().cloned()
    }

    /// Số lần `send` được gọi (kể cả mail trùng khoá bị bỏ, mail bị từ chối).
    pub fn calls(&self) -> usize {
        lock(&self.state).calls
    }

    /// Quên mọi mail đã nhận.
    pub fn clear(&self) {
        let mut state = lock(&self.state);
        state.sent.clear();
        state.calls = 0;
    }

    /// Bật thì mọi mail có `reply_to` bị từ chối như máy chủ mail không chấp nhận địa chỉ đó.
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
