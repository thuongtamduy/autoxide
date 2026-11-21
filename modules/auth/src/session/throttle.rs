//! Login throttling: chặn đoán mật khẩu bằng cách thử liên tục.
//!
//! Đăng nhập bằng email (D-20261121-login-throttle-pair): đếm theo CẶP email + địa chỉ IP với
//! `max_failures`, cộng một trần rộng hơn (`ACCOUNT_FACTOR` lần) cho cả email. Người khác chỉ
//! khoá được cặp của chính họ; muốn khoá cả email phải thử sai từ nhiều địa chỉ, và trần đó vẫn
//! chặn đoán mật khẩu phân tán. Không biết IP thì cặp là "email + không rõ" (như trước).
//!
//! Đếm lần đăng nhập sai theo email (kể cả email không tồn tại — để không lộ email
//! nào có thật). Quá `max_failures` lần trong một `period` thì khoá đăng nhập của email
//! đó trong `period`, kể cả khi mật khẩu đúng. Đếm trong database (bảng `auth_throttle`,
//! bằng `Throttle` của kernel, D-116) nên mọi máy chủ dùng chung một con số, và nhiều lần thử
//! cùng lúc không vượt được số lần cho phép (D-079).

use std::net::IpAddr;
use std::time::Duration;

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::throttle::{Outcome, Throttle};

/// Trần lần sai của cả một email so với trần của một cặp email + IP.
const ACCOUNT_FACTOR: u32 = 10;

pub(crate) struct LoginThrottle {
    inner: Throttle,
    /// Trần của cả email khi đăng nhập (mọi địa chỉ cộng lại).
    account: Throttle,
}

impl LoginThrottle {
    pub(crate) fn new(max_failures: u32, period: Duration) -> Result<Self, Error> {
        Ok(LoginThrottle {
            inner: Throttle::new("auth", max_failures, period)?,
            account: Throttle::new("auth", max_failures.saturating_mul(ACCOUNT_FACTOR), period)?,
        })
    }

    /// Xin một lượt đăng nhập cho `email` từ địa chỉ `ip` (đếm trước khi kiểm, như
    /// [`reserve`](Self::reserve)): cặp email + IP rồi cả email.
    pub(crate) async fn reserve_login(
        &self,
        tenant: &TenantContext,
        email: &str,
        ip: Option<IpAddr>,
    ) -> Result<(), Error> {
        refuse(self.inner.hit(tenant, &pair_key(email, ip)).await?)?;
        refuse(self.account.hit(tenant, &key(email)).await?)
    }

    /// Trả lại lượt vừa xin bằng [`reserve_login`](Self::reserve_login).
    pub(crate) async fn refund_login(
        &self,
        tenant: &TenantContext,
        email: &str,
        ip: Option<IpAddr>,
    ) -> Result<(), Error> {
        self.inner.refund(tenant, &pair_key(email, ip)).await?;
        self.account.refund(tenant, &key(email)).await
    }

    /// Đăng nhập đúng: quên các lần sai của cặp và của email.
    pub(crate) async fn clear_login(
        &self,
        tenant: &TenantContext,
        email: &str,
        ip: Option<IpAddr>,
    ) -> Result<(), Error> {
        self.inner.clear(tenant, &pair_key(email, ip)).await?;
        self.account.clear(tenant, &key(email)).await
    }

    /// Lỗi 429 `auth.login_throttled` nếu đăng nhập của `email` đang bị khoá (không đếm).
    #[cfg(test)]
    pub(crate) async fn check(&self, tenant: &TenantContext, email: &str) -> Result<(), Error> {
        refuse(self.inner.check(tenant, &key(email)).await?)
    }

    /// Xin một lượt thử cho `email`, TRƯỚC khi kiểm mật khẩu hay mã: đếm lượt này, rồi
    /// từ chối nếu email đang (hoặc vừa) bị khoá. Đếm trước rồi mới kiểm nên nhiều request
    /// bắn cùng lúc không vượt được số lần cho phép (D-079). Thử đúng thì gọi
    /// [`clear`](Self::clear).
    pub(crate) async fn reserve(&self, tenant: &TenantContext, email: &str) -> Result<(), Error> {
        refuse(self.inner.hit(tenant, &key(email)).await?)
    }

    /// Đăng nhập đúng: quên các lần sai trước đó.
    pub(crate) async fn clear(&self, tenant: &TenantContext, email: &str) -> Result<(), Error> {
        self.inner.clear(tenant, &key(email)).await
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

/// Khoá đếm của cặp email + địa chỉ (`-`: không biết địa chỉ).
fn pair_key(email: &str, ip: Option<IpAddr>) -> String {
    let ip = ip.map_or_else(|| "-".to_owned(), |ip| ip.to_string());
    format!("login-ip:{}|{ip}", email.trim().to_lowercase())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use autoxide_kernel::error::ErrorKind;
    use autoxide_testkit::TestDatabase;

    async fn throttle(test: &TestDatabase, max_failures: u32) -> (LoginThrottle, TenantContext) {
        let database = crate::testing::database(test).await;
        let throttle = LoginThrottle::new(max_failures, Duration::from_secs(15 * 60)).unwrap();
        (throttle, TenantContext::default_on(database))
    }

    #[tokio::test]
    async fn sai_qua_so_lan_thi_khoa_roi_dung_thi_quen() {
        TestDatabase::run(async |test| {
            let (throttle, tenant) = throttle(test, 3).await;

            // Ba lượt đầu được kiểm (dù sai); lượt thứ tư bị khoá.
            for _ in 0..3 {
                throttle.reserve(&tenant, "Ada@Example.com").await.unwrap();
            }
            let error = throttle.reserve(&tenant, " ADA@example.com").await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::TooManyRequests);
            assert_eq!(error.code(), "auth.login_throttled");
            let wait = error.retry_after().unwrap();
            assert!((899..=900).contains(&wait), "{wait}");
            // Email khác không bị ảnh hưởng.
            throttle.check(&tenant, "bob@example.com").await.unwrap();

            throttle.clear(&tenant, "ada@example.com").await.unwrap();
            throttle.check(&tenant, "ada@example.com").await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Nhiều lần thử bắn cùng lúc: không bao giờ quá `max_failures` lượt được kiểm, và
    /// email bị khoá sau đó — trên cả hai database (D-079).
    #[tokio::test]
    async fn thu_dong_thoi_khong_vuot_so_lan_cho_phep() {
        TestDatabase::run(async |test| {
            let (throttle, tenant) = throttle(test, 5).await;
            let throttle = Arc::new(throttle);

            let attempts: Vec<_> = (0..40)
                .map(|_| {
                    let (throttle, tenant) = (throttle.clone(), tenant.clone());
                    let tenant = tenant.clone();
                    tokio::spawn(async move { throttle.reserve(&tenant, "ada@example.com").await })
                })
                .collect();
            let mut allowed = 0;
            for attempt in attempts {
                if attempt.await.unwrap().is_ok() {
                    allowed += 1;
                }
            }

            assert!(allowed <= 5, "{allowed} attempts got through");
            assert!(throttle.check(&tenant, "ada@example.com").await.is_err());
        })
        .await
        .unwrap();
    }
}
