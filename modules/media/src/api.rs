//! Hợp đồng của module `media` cho module khác (vd module nội dung sắp tới): lấy MỘT hay
//! NHIỀU item theo ID một lần — trang có 50 ảnh không phải gọi 50 lần.
//!
//! Lấy bằng `context.require::<dyn Media>()` trong `Module::start` (đăng ký SAU `media`).
//! Item trong thùng rác coi như không có. File này không tham chiếu phần bên trong module.

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
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
    /// `alt` mặc định viết từ gợi ý của AI (Luật 134/2025/QH15, D-362): giao diện gắn nhãn.
    pub alt_ai_assisted: bool,
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
    /// `url` của item kèm `&w=<width>`: bản JPEG (PNG với ảnh có vùng trong suốt).
    pub url: String,
    /// Bản WebP cùng cỡ (`url` kèm `&format=webp`), nhẹ hơn nhiều; `None` khi chưa làm (ảnh tải
    /// lên trước khi có WebP, tới khi chạy `autoxide task media.refresh_variants`).
    pub webp_url: Option<String>,
    /// Bản AVIF cùng cỡ (`url` kèm `&format=avif`, bước 39 B8); `None` khi chưa làm hay máy chủ
    /// không bật `AUTOXIDE_MEDIA_AVIF`.
    pub avif_url: Option<String>,
}

/// Ảnh chuẩn bị để gửi cho mô hình AI ([`Media::ai_image`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AiImage {
    /// `image/jpeg` hay `image/png`.
    pub media_type: String,
    pub bytes: Vec<u8>,
}

impl AiImage {
    pub fn new(media_type: impl Into<String>, bytes: Vec<u8>) -> Self {
        AiImage { media_type: media_type.into(), bytes }
    }
}

#[async_trait::async_trait]
pub trait Media: Send + Sync {
    /// Các item có ID trong `ids` và không trong thùng rác, theo thứ tự của `ids` (ID không
    /// có thì bỏ qua). Hai câu query cho mọi ID (item, ảnh thu nhỏ). Quá [`MAX_IDS`] ID thì
    /// lỗi.
    async fn find_many(&self, tenant: &TenantContext, ids: &[Uuid])
    -> Result<Vec<MediaRef>, Error>;

    /// Như [`find_many`](Self::find_many), với `alt` bằng ngôn ngữ `locale` (mã BCP 47, vd ngôn
    /// ngữ của bài đang đọc): bản dịch nếu item có, không thì `alt` mặc định (D-202). Thêm một
    /// câu query cho mọi ID.
    async fn find_many_in(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<Vec<MediaRef>, Error> {
        let _ = locale;
        self.find_many(tenant, ids).await
    }

    /// Item `id`, hay `None` nếu không có / đang trong thùng rác.
    async fn find(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<MediaRef>, Error> {
        Ok(self.find_many(tenant, &[id]).await?.into_iter().next())
    }

    /// Người tải lên của các item có ID trong `ids` và không trong thùng rác (`None`: hệ thống tải
    /// lên); ID không có thì không có mặt. Để module khác kiểm tệp đính kèm thuộc người gửi (vd
    /// phiếu hỗ trợ, D-202). Bộ cài đặt cũ không biết người tải lên: mọi item là `None`.
    async fn uploaders(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Option<Uuid>)>, Error> {
        Ok(self.find_many(tenant, ids).await?.into_iter().map(|item| (item.id, None)).collect())
    }

    /// Ảnh `id` để module khác gửi cho mô hình AI (D-109, vd `catalog` tạo sản phẩm nháp từ ảnh —
    /// D-229): ảnh thu nhỏ do máy chủ làm, cạnh dài tối đa 1280 px, đã bỏ metadata (EXIF) — không
    /// bao giờ là file gốc. `None` khi không có hay đang trong thùng rác. Không phải ảnh đọc được:
    /// lỗi `media.ai_unsupported_image`; ảnh riêng tư khi máy chủ không cho gửi ảnh riêng tư:
    /// `media.ai_private_not_allowed`. Đọc nơi lưu: gọi NGOÀI transaction ghi.
    ///
    /// Cài đặt mặc định trả `None` (bản giả trong test của module khác không phải cài).
    async fn ai_image(&self, tenant: &TenantContext, id: Uuid) -> Result<Option<AiImage>, Error> {
        let _ = (tenant, id);
        Ok(None)
    }
}
