//! Hợp đồng của module `notes` cho module khác: chỉ những gì ở đây được dùng từ bên
//! ngoài (`autoxide_notes::api::…`). Module khác lấy trait lúc khởi động:
//!
//! ```text
//! let notes = context.require::<dyn autoxide_notes::api::Notes>()?;
//! ```
//!
//! Module đó phải đăng ký SAU `notes` trong `crates/app/src/modules.rs`.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Tóm tắt một ghi chú cho module khác (không kèm nội dung).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSummary {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub title: String,
}

/// Tra ghi chú, vd để module khác kiểm một ID ghi chú có thật không (không có foreign
/// key xuyên module — luật kiến trúc 4).
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này
// đổi hàm `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Notes: Send + Sync {
    /// Ghi chú có ID `id` của khách `tenant`, hoặc `None` nếu không có.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<NoteSummary>, Error>;
}
