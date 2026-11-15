//! Hợp đồng của module `content` cho module khác.
//!
//! Hiện chỉ có [`LegacyMenus`]: đọc menu cũ của `content` (bước 17) để module `menus` nhập sang
//! (D-238). Menu của `content` đã deprecated, sunset 2027-05-14; qua ngày đó trait này bị bỏ cùng
//! endpoint menu cũ. Lấy bằng `context.require::<dyn LegacyMenus>()` trong `Module::start` (đăng
//! ký SAU `content`). File này không tham chiếu phần bên trong module.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

/// Một menu cũ: khoá, ngôn ngữ và cây mục (đã theo thứ tự).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMenu {
    pub key: String,
    pub locale: String,
    pub items: Vec<LegacyMenuItem>,
}

/// Một mục của menu cũ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyMenuItem {
    pub label: String,
    pub target: LegacyTarget,
    pub children: Vec<LegacyMenuItem>,
}

/// Đích của mục menu cũ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyTarget {
    /// Bài viết hay trang (loại `content.entry` của sổ đối tượng).
    Entry(Uuid),
    /// Danh mục hay thẻ (loại `content.term`).
    Term(Uuid),
    /// Địa chỉ: `https://`, `http://`, `mailto:` hay đường dẫn `/…`.
    Url(String),
}

/// Đọc menu cũ của `content`.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait LegacyMenus: Send + Sync {
    /// Mọi menu cũ của khách `tenant`, theo khoá rồi ngôn ngữ.
    async fn legacy_menus(&self, tenant: &TenantContext) -> Result<Vec<LegacyMenu>, Error>;
}
