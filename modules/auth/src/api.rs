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
use autoxide_kernel::time::Timestamp;
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
    pub disabled_at: Option<Timestamp>,
    /// Lúc chủ tài khoản xác nhận `email` là của mình (qua mail); `None`: chưa xác minh.
    pub email_verified_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Nhân sự (có role) hay người mua của khách (không permission nào) — bước 34, D-309.
    pub realm: autoxide_kernel::access::Realm,
    /// Ngôn ngữ ưa thích (`en`, `vi`) cho mail, thông báo gửi chậm; `None`: theo request (D-308).
    pub locale: Option<String>,
    /// Múi giờ IANA ưa thích; `None`: chưa đặt.
    pub time_zone: Option<String>,
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

/// Quản trị viên của một khách: người dùng còn hoạt động có role mang mọi permission (role `admin`).
/// Module `notifications` dùng để gửi thông báo của nền tảng (thuê bao quá hạn, sắp chạm hạn mức…)
/// tới người quản trị mọi khách (bước 35, D-348).
#[async_trait::async_trait]
pub trait TenantAdmins: Send + Sync {
    /// Quản trị viên còn hoạt động của khách `tenant`, theo thứ tự ID.
    async fn admins(&self, tenant: &TenantContext) -> Result<Vec<User>, Error>;
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

/// Lời mời mới cho lệnh `autoxide user invite` và việc cấp phát khách (bước 35).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NewInvitation {
    pub email: String,
    pub name: Option<String>,
    /// Tên các role của khách gán khi người được mời chấp nhận.
    pub roles: Vec<String>,
    /// Nút của các role; `None`: nút gốc.
    pub org_id: Option<Uuid>,
}

impl NewInvitation {
    pub fn new(email: impl Into<String>, roles: Vec<String>) -> Self {
        NewInvitation { email: email.into(), name: None, roles, org_id: None }
    }

    pub fn at(mut self, org_id: Uuid) -> Self {
        self.org_id = Some(org_id);
        self
    }

    /// Tên hiển thị gợi ý cho người được mời.
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

/// Lời mời vừa tạo; mail đi trong ít phút (job).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct InvitationSummary {
    pub id: Uuid,
    pub email: String,
    pub expires_at: Timestamp,
}

impl InvitationSummary {
    pub fn new(id: Uuid, email: impl Into<String>, expires_at: Timestamp) -> Self {
        InvitationSummary { id, email: email.into(), expires_at }
    }
}

/// Mời người dùng qua email (D-312). Không kiểm permission — bên gọi chịu trách nhiệm (lệnh CLI
/// chạy với quyền của người vận hành, cấp phát của nền tảng).
#[async_trait::async_trait]
pub trait Invitations: Send + Sync {
    /// Mời `invitation.email` vào khách `tenant`. Lỗi: email sai, role không có
    /// (`validation_failed`).
    async fn invite(
        &self,
        tenant: &TenantContext,
        invitation: NewInvitation,
    ) -> Result<InvitationSummary, Error>;
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

/// Mã một lần gửi tới số điện thoại đã gửi đi (bước 47, đăng nhập bằng số).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneCodeSent {
    /// ID của mã; người dùng gửi lại cùng mã họ nhận được.
    pub id: Uuid,
    pub expires_at: Timestamp,
}

/// Gửi và kiểm mã một lần qua SMS cho đăng nhập bằng số điện thoại (bước 47,
/// D-20261120-phone-accounts). `compliance` cài (giữ quy tắc số được nhận, giới hạn gửi, bên gửi SMS)
/// và cắm vào bằng [`AuthExtensions::set_phone_codes`] vì đăng ký sau `auth`.
#[async_trait::async_trait]
pub trait PhoneCodes: Send + Sync {
    /// Gửi mã tới số `phone` (chưa chuẩn hoá). Lỗi khi số không được nhận, gửi quá nhiều, SMS hỏng.
    async fn send(
        &self,
        tenant: &TenantContext,
        phone: &str,
        ip: Option<std::net::IpAddr>,
    ) -> Result<PhoneCodeSent, Error>;

    /// Kiểm mã `code` của `id`: đúng, còn hạn, chưa dùng → số đã xác thực (dùng một lần).
    async fn verify(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<autoxide_kernel::sms::PhoneNumber, Error>;

    /// Ghi số vừa xác thực là số của tài khoản `user_id` (cho module khác biết qua
    /// `AccountVerification`). Bỏ qua, không lỗi, khi quy tắc của khách không cho thêm tài khoản dùng số.
    async fn record(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        phone: &autoxide_kernel::sms::PhoneNumber,
    ) -> Result<(), Error>;
}

/// Điểm cắm của `auth` cho module đăng ký sau (giống `OrderExtensions` của `orders`).
pub trait AuthExtensions: Send + Sync {
    /// Cắm bộ gửi / kiểm mã cho đăng nhập bằng số. Cắm lần hai: lỗi.
    fn set_phone_codes(&self, codes: std::sync::Arc<dyn PhoneCodes>) -> Result<(), Error>;
}
