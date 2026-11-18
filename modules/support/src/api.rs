//! Hợp đồng của module `support` cho module khác (`autoxide_support::api::…`). Module dùng phải
//! đăng ký SAU `support` trong `crates/app/src/modules.rs` và lấy trait lúc khởi động:
//!
//! ```text
//! let intake = context.require::<dyn autoxide_support::api::SupportIntake>()?;
//! ```

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Người hỏi của một phiếu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Requester {
    /// Một tài khoản (người mua đã đăng nhập).
    User(Uuid),
    /// Số điện thoại người hỏi đã chứng minh (vd OTP của trang xem đơn, bước 45), dạng E.164.
    VerifiedPhone(String),
    /// Email người hỏi đã chứng minh.
    VerifiedEmail(String),
}

/// Một phiếu module khác mở thay người mua (vd nút "mở hỗ trợ" trên trang đơn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTicket {
    /// Nút nhận phiếu (cửa hàng, gian hàng); nút gốc = ID của khách.
    pub org_id: Uuid,
    pub requester: Requester,
    /// Tên hiển thị của người hỏi (rỗng: "Customer").
    pub requester_name: String,
    /// Mã loại (`order`, `product`, `payment`, `account`, `other` hay loại khách tự khai).
    pub category: String,
    pub subject: String,
    pub body: String,
    /// Bản ghi liên quan, dạng `<kind>:<id>` (vd `orders.order:<id>`).
    pub links: Vec<String>,
    /// Khoá chống trùng của bên gọi: gọi lại cùng `reference` trả phiếu cũ, không tạo phiếu mới.
    pub reference: String,
}

/// Phiếu vừa mở (hay đã có với cùng `reference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRef {
    pub id: Uuid,
    /// Số phiếu hiển thị cho người mua.
    pub number: String,
    /// Token xem phiếu của người mua vãng lai — chỉ có ở lần tạo đầu tiên (database chỉ giữ bản
    /// băm); `None` với người hỏi có tài khoản hay khi gọi lại.
    pub access_token: Option<String>,
}

/// Mở phiếu hỗ trợ thay người mua.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro đổi hàm `async`
// thành hàm trả `Box<dyn Future>`.
#[async_trait::async_trait]
pub trait SupportIntake: Send + Sync {
    /// Tạo phiếu; cùng `reference` → phiếu cũ.
    async fn open(&self, tenant: &TenantContext, ticket: NewTicket) -> Result<TicketRef, Error>;
}
