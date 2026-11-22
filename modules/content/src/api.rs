//! Hợp đồng của module `content` cho module khác.
//!
//! Hiện chỉ có [`LegacyMenus`]: đọc menu cũ của `content` (bước 17) để module `menus` nhập sang
//! (D-238). Menu của `content` đã deprecated, sunset 2027-05-14; qua ngày đó trait này bị bỏ cùng
//! endpoint menu cũ. Lấy bằng `context.require::<dyn LegacyMenus>()` trong `Module::start` (đăng
//! ký SAU `content`). File này không tham chiếu phần bên trong module.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
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

/// Ảnh chụp bản đã đăng của một bài / trang (bước 37 C1): đủ để module khác (vd văn bản pháp lý
/// của `compliance`) chép lại thành bản bất biến mà không đọc bảng của `content`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PublishedSnapshot {
    pub entry_id: Uuid,
    pub title: String,
    pub locale: String,
    /// Thân ProseMirror JSON đã đăng.
    pub body: serde_json::Value,
    pub plain_text: String,
    /// Revision của bản đã đăng (không phải bản nháp).
    pub revision_id: Uuid,
    pub published_at: Timestamp,
}

/// Đọc bản đã đăng của bài / trang. Lấy bằng `context.require::<dyn PublishedEntries>()` (đăng ký
/// SAU `content`).
#[async_trait::async_trait]
pub trait PublishedEntries: Send + Sync {
    /// Bản đã đăng của mục `entry_id`; chưa đăng hay không có → `None`.
    async fn published_snapshot(
        &self,
        tenant: &TenantContext,
        entry_id: Uuid,
    ) -> Result<Option<PublishedSnapshot>, Error>;
}

/// Phần của module khác trong khung website (`GET /api/v1/content/public/site`, bước 67, D-234): vd
/// cấu hình đồng ý cookie và mã theo dõi của `compliance`. `content` đăng ký trước các module đó nên
/// không `require` được chúng; module sau tự cắm vào qua [`SiteExtensions`] (lấy bằng
/// `context.require::<dyn SiteExtensions>()` trong `Module::start`). Kết quả nằm ở
/// `extensions.<module>` của câu trả lời, cùng `ETag` và `max-age`. Chỉ trả dữ liệu công khai.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SiteExtension: Send + Sync {
    /// Phần của module cho website `request.site` ở ngôn ngữ `request.locale`; `None`: không có gì.
    /// Gọi khi khung website được tính lại (tối đa mỗi 30 giây mỗi website), không mỗi request.
    async fn site(
        &self,
        request: &autoxide_kernel::blocks::BlockRequest<'_>,
    ) -> Result<Option<serde_json::Value>, Error>;
}

/// Sổ phần mở rộng của khung website.
pub trait SiteExtensions: Send + Sync {
    /// Cắm phần của `module` (tên module, mỗi module một lần) vào khung website.
    fn register(
        &self,
        module: &'static str,
        extension: std::sync::Arc<dyn SiteExtension>,
    ) -> Result<(), Error>;
}
