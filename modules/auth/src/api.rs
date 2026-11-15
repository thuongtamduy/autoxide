//! Hợp đồng của module `auth` cho module khác: chỉ những gì ở đây được dùng từ
//! bên ngoài (`autoxide_auth::api::…`).
//!
//! Module khác lấy trait lúc khởi động:
//!
//! ```text
//! let users = context.require::<dyn autoxide_auth::api::Users>()?;
//! ```
//!
//! Module đó phải đăng ký SAU `auth` trong `crates/app/src/modules.rs`.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Một tài khoản người dùng. Không bao giờ mang mật khẩu hay hash của nó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: Uuid,
    /// Email đã chuẩn hoá (bỏ khoảng trắng hai đầu, chữ thường). Duy nhất.
    pub email: String,
    /// Tên hiển thị.
    pub name: String,
    /// Lúc tài khoản bị khoá; `None` là đang hoạt động.
    pub disabled_at: Option<DateTime<Utc>>,
    /// Lúc chủ tài khoản xác nhận `email` là của mình (qua mail); `None`: chưa xác minh.
    pub email_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    /// Tài khoản còn hoạt động (chưa bị khoá).
    pub fn is_active(&self) -> bool {
        self.disabled_at.is_none()
    }
}

/// Tra người dùng. Module khác dùng trait này để kiểm một ID người dùng có tồn
/// tại không (không có foreign key xuyên module — D-025) và để hiển thị tên.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro
// này đổi hàm `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Users: Send + Sync {
    /// Người dùng có ID `id` của khách `tenant`, hoặc `None` nếu không có.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<User>, Error>;
}

/// Dữ liệu để tạo một tài khoản.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub name: String,
    /// `None`: tài khoản chưa có mật khẩu, chưa đăng nhập bằng mật khẩu được.
    pub password: Option<String>,
    /// Gán role `admin` (mọi permission).
    pub admin: bool,
}

/// Tạo tài khoản. Binary dùng trait này cho lệnh `autoxide user create`; nó không
/// kiểm permission — bên gọi chịu trách nhiệm (lệnh CLI chạy với quyền của người
/// vận hành máy chủ).
#[async_trait::async_trait]
pub trait Accounts: Send + Sync {
    /// Tạo tài khoản. Lỗi: email sai hoặc đã có (`auth.email_taken`), mật khẩu không
    /// đạt luật độ dài (`validation_failed`).
    async fn create(&self, tenant: &TenantContext, account: NewAccount) -> Result<User, Error>;
}

/// Việc của người vận hành máy chủ khi không ai đăng nhập được để làm qua API. Binary dùng cho
/// lệnh `autoxide user reset-mfa`; không kiểm permission — lệnh CLI chạy với quyền của người
/// vận hành (D-159).
#[async_trait::async_trait]
pub trait AccountRecovery: Send + Sync {
    /// Tắt xác thực hai bước (TOTP và recovery code) của tài khoản có email `email`, như quản
    /// trị viên làm qua `DELETE /api/v1/auth/users/{id}/mfa`: cho admin duy nhất mất thiết bị.
    /// Lỗi: không có tài khoản này (`auth.account_not_found`).
    async fn reset_mfa(&self, tenant: &TenantContext, email: &str) -> Result<User, Error>;

    /// Gỡ mọi passkey của tài khoản có email `email`, như quản trị viên làm qua
    /// `DELETE /api/v1/auth/users/{id}/passkeys`: cho admin duy nhất mất thiết bị (đăng nhập lại
    /// bằng mật khẩu). Trả tài khoản và số passkey đã gỡ. Lỗi: `auth.account_not_found`.
    async fn remove_passkeys(
        &self,
        tenant: &TenantContext,
        email: &str,
    ) -> Result<(User, u64), Error>;
}
