//! Hợp đồng của module `customers` cho module khác (bước 48–49, `wholesale`, `analytics`…): chỉ
//! những gì ở đây được dùng từ bên ngoài (`autoxide_customers::api::…`). File này không tham chiếu
//! phần bên trong module.
//!
//! Module đăng ký SAU `customers` lấy trait lúc khởi động:
//!
//! ```text
//! let customers = context.require::<dyn autoxide_customers::api::Customers>()?;
//! ```
//!
//! `customers` tự cắm nhóm khách vào `orders` (`OrderExtensions::set_buyer_groups`), nên `orders`
//! không cần gọi trait này.

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tên event `customers` phát.
pub mod events {
    pub const PROFILE_CREATED: &str = "customers.profile_created";
    pub const PROFILE_UPDATED: &str = "customers.profile_updated";
    pub const PROFILE_LINKED: &str = "customers.profile_linked";
    pub const GROUP_MEMBERSHIP_CHANGED: &str = "customers.group_membership_changed";
}

/// Tóm tắt một hồ sơ khách cho module khác. Liên hệ đã che (D-226): không bao giờ có số điện thoại
/// hay email đầy đủ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CustomerSummary {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    /// Nút nơi khách đăng ký / mua lần đầu.
    pub org_id: OrgId,
    pub full_name: Option<String>,
    /// `09•• ••• 412`.
    pub phone_masked: Option<String>,
    /// `n•••@gmail.com`.
    pub email_masked: Option<String>,
    /// Hạng thành viên hiện tại.
    pub tier: Option<String>,
    pub group_ids: Vec<Uuid>,
    /// Tổng chi: Σ tiền hàng của đơn đã hoàn tất − hoàn tiền (D-226). `None` khi chưa mua.
    pub total_spent: Option<Money>,
    pub orders_count: i64,
}

/// Đọc hồ sơ khách.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Customers: Send + Sync {
    /// Hồ sơ gắn với tài khoản người mua `user_id`, hoặc `None` (không tạo mới).
    async fn find_by_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Option<CustomerSummary>, Error>;

    /// ID các nhóm khách của tài khoản `user_id` (rỗng khi chưa có hồ sơ).
    async fn groups_of_user(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Vec<Uuid>, Error>;
}
