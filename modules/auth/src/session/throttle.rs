//! Login throttling: chặn đoán mật khẩu bằng cách thử liên tục.
//!
//! Đếm lần đăng nhập sai theo email (kể cả email không tồn tại — để không lộ email
//! nào có thật). Quá `max_failures` lần trong một `period` thì khoá đăng nhập của email
//! đó trong `period`, kể cả khi mật khẩu đúng. Đếm trong database (bảng `auth_throttle`,
//! bằng `Throttle` của kernel, D-110) nên mọi máy chủ dùng chung một con số, và nhiều lần thử
//! cùng lúc không vượt được số lần cho phép (D-079).

use std::time::Duration;

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::throttle::{Outcome, Throttle};

pub(crate) struct LoginThrottle {
    inner: Throttle,
}

impl LoginThrottle {
    pub(crate) fn new(
        database: Database,
        max_failures: u32,
        period: Duration,
    ) -> Result<Self, Error> {
        Ok(LoginThrottle { inner: Throttle::new(database, "auth", max_failures, period)? })
    }

    /// Lỗi 429 `auth.login_throttled` nếu đăng nhập của `email` đang bị khoá (không đếm).
    #[cfg(test)]
    pub(crate) async fn check(&self, email: &str) -> Result<(), Error> {
        refuse(self.inner.check(&key(email)).await?)
    }

    /// Xin một lượt thử cho `email`, TRƯỚC khi kiểm mật khẩu hay mã: đếm lượt này, rồi
    /// từ chối nếu email đang (hoặc vừa) bị khoá. Đếm trước rồi mới kiểm nên nhiều request
    /// bắn cùng lúc không vượt được số lần cho phép (D-079). Thử đúng thì gọi
    /// [`clear`](Self::clear).
    pub(crate) async fn reserve(&self, email: &str) -> Result<(), Error> {
        refuse(self.inner.hit(&key(email)).await?)
    }

    /// Trả lại lượt vừa xin bằng [`reserve`](Self::reserve) khi lần thử đó không sai (vd
    /// mật khẩu đúng, còn chờ mã MFA): người dùng thật không bị tính oan.
    pub(crate) async fn refund(&self, email: &str) -> Result<(), Error> {
        self.inner.refund(&key(email)).await
    }

    /// Đăng nhập đúng: quên các lần sai trước đó.
    pub(crate) async fn clear(&self, email: &str) -> Result<(), Error> {
        self.inner.clear(&key(email)).await
    }
}

fn refuse(outcome: Outcome) -> Result<(), Error> {
    match outcome {
        Outcome::Allowed => Ok(()),
        Outcome::Limited { retry_after } => Err(Error::too_many_requests(
            "auth.login_throttled",
            "Too many failed sign-in attempts for this email. Wait {seconds} seconds, then try \
             again.",
        )
        .with_param("seconds", retry_after)
        .with_retry_after(retry_after)),
    }
}

/// Khoá đếm của một email (đã chuẩn hoá); kernel băm trước khi lưu.
fn key(email: &str) -> String {
    format!("login:{}", email.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_testkit::TestDatabase;

    async fn throttle(test: &TestDatabase, max_failures: u32) -> LoginThrottle {
        let database = crate::testing::database(test).await;
        LoginThrottle::new(database, max_failures, Duration::from_secs(15 * 60)).unwrap()
    }

    #[tokio::test]
    async fn sai_qua_so_lan_thi_khoa_roi_dung_thi_quen() {
        TestDatabase::run(async |test| {
            let throttle = throttle(test, 3).await;

            // Ba lượt đầu được kiểm (dù sai); lượt thứ tư bị khoá.
            for _ in 0..3 {
                throttle.reserve("Ada@Example.com").await.unwrap();
            }
            let error = throttle.reserve(" ADA@example.com").await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::TooManyRequests);
            assert_eq!(error.code(), "auth.login_throttled");
            let wait = error.retry_after().unwrap();
            assert!((899..=900).contains(&wait), "{wait}");
            // Email khác không bị ảnh hưởng.
            throttle.check("bob@example.com").await.unwrap();

            throttle.clear("ada@example.com").await.unwrap();
            throttle.check("ada@example.com").await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Nhiều lần thử bắn cùng lúc: không bao giờ quá `max_failures` lượt được kiểm, và
    /// email bị khoá sau đó — trên cả hai database (D-079).
    #[tokio::test]
    async fn thu_dong_thoi_khong_vuot_so_lan_cho_phep() {
        TestDatabase::run(async |test| {
            let throttle = Arc::new(throttle(test, 5).await);

            let attempts: Vec<_> = (0..40)
                .map(|_| {
                    let throttle = throttle.clone();
                    tokio::spawn(async move { throttle.reserve("ada@example.com").await })
                })
                .collect();
            let mut allowed = 0;
            for attempt in attempts {
                if attempt.await.unwrap().is_ok() {
                    allowed += 1;
                }
            }

            assert!(allowed <= 5, "{allowed} attempts got through");
            assert!(throttle.check("ada@example.com").await.is_err());
        })
        .await
        .unwrap();
    }
}
