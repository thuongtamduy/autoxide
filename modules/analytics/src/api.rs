//! Hợp đồng của module `analytics` cho module khác (đăng ký SAU `analytics`):
//! `context.require::<dyn Analytics>()`. Ví dụ: `catalog` hỏi "thịnh hành" (đối tượng xem nhiều),
//! `customers` hỏi "sản phẩm vừa xem" của một người (chỉ có khi người đó đồng ý theo dõi theo người).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use uuid::Uuid;

/// Mục đích trong sổ đồng ý của `compliance` cho phép gắn sự kiện phân tích với người dùng.
pub const PERSONAL_PURPOSE: &str = "analytics.personal";

/// Loại sự kiện "xem trang" (website gửi kèm `object` khi trang là một đối tượng).
pub const PAGE_VIEW: &str = "analytics.page_view";

/// Một đối tượng và số lần được xem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectViews {
    /// `<loại>:<id>`, vd `catalog.product:0190…`.
    pub object: String,
    /// Phần `<id>`.
    pub id: String,
    /// Số sự kiện nói tới đối tượng.
    pub views: i64,
    /// Tổng khách của từng ngày.
    pub visitors: i64,
}

// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait Analytics: Send + Sync {
    /// Đối tượng loại `kind` (vd `catalog.product`) được xem nhiều nhất trên website `site` (`None`:
    /// website của cả khách) trong `days` ngày gần nhất (1–400), nhiều nhất trước, tối đa `limit`
    /// (1–100). Đọc số gộp, không quét sự kiện thô.
    async fn top_objects(
        &self,
        tenant: &TenantContext,
        site: Option<OrgId>,
        kind: &str,
        days: u32,
        limit: u32,
    ) -> Result<Vec<ObjectViews>, Error>;

    /// Đối tượng loại `kind` người dùng `user_id` vừa xem, mới nhất trước, không trùng, tối đa
    /// `limit` (1–100). Rỗng khi người đó không đồng ý theo dõi theo người (hay đã rút).
    async fn recently_viewed(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
        kind: &str,
        limit: u32,
    ) -> Result<Vec<String>, Error>;
}
