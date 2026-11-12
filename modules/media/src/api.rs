//! Hợp đồng của module `media` cho module khác (vd module nội dung sắp tới): lấy MỘT hay
//! NHIỀU item theo ID một lần — trang có 50 ảnh không phải gọi 50 lần.
//!
//! Lấy bằng `context.require::<dyn Media>()` trong `Module::start` (đăng ký SAU `media`).
//! Item trong thùng rác coi như không có. File này không tham chiếu phần bên trong module.

use autoxide_kernel::error::Error;
use uuid::Uuid;

/// Số ID tối đa mỗi lần gọi [`Media::find_many`].
pub const MAX_IDS: usize = 500;

/// Ai tải được nội dung không cần đăng nhập.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaVisibility {
    /// Chỉ người có `media.read`.
    Private,
    /// Ai có link cũng tải được.
    Public,
}

/// Những gì module khác cần để hiển thị một item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaRef {
    pub id: Uuid,
    pub visibility: MediaVisibility,
    /// Loại nội dung, vd `image/webp`.
    pub content_type: String,
    /// Kích thước (pixel), chỉ ảnh raster có.
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Mô tả thay thế (`alt` của thẻ `<img>`); rỗng nếu chưa đặt.
    pub alt: String,
    /// Đường dẫn tải nội dung, kèm phiên bản (`/api/v1/media/<id>/content?v=<phiên bản>`):
    /// đổi khi nội dung đổi, nên cache được lâu. Không cần đăng nhập khi `Public`.
    pub url: String,
    /// Ảnh thu nhỏ đã làm xong, hẹp trước (cho `srcset`); rỗng nếu không phải ảnh JPEG, PNG,
    /// GIF, WebP hay chưa làm xong.
    pub variants: Vec<MediaVariant>,
}

/// Một ảnh thu nhỏ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaVariant {
    pub width: i32,
    pub height: i32,
    /// `url` của item kèm `&w=<width>`.
    pub url: String,
}

#[async_trait::async_trait]
pub trait Media: Send + Sync {
    /// Các item có ID trong `ids` và không trong thùng rác, theo thứ tự của `ids` (ID không
    /// có thì bỏ qua). Hai câu query cho mọi ID (item, ảnh thu nhỏ). Quá [`MAX_IDS`] ID thì
    /// lỗi.
    async fn find_many(&self, ids: &[Uuid]) -> Result<Vec<MediaRef>, Error>;

    /// Item `id`, hay `None` nếu không có / đang trong thùng rác.
    async fn find(&self, id: Uuid) -> Result<Option<MediaRef>, Error> {
        Ok(self.find_many(&[id]).await?.into_iter().next())
    }
}
