//! Hợp đồng của module `files`:
//!
//! - trait chung `autoxide_kernel::storage::Storage` (nơi lưu); module khác lấy bằng
//!   `context.require::<dyn Storage>()` (đăng ký SAU `files`);
//! - trait [`Files`]: đọc một file người dùng đã tải lên qua `/api/v1/files`, với đúng luật xem của
//!   `files` (chủ file hay `files.manage`). Module khác dùng nó để nhận một file người dùng đã tải
//!   lên (vd `catalog` chép file của hàng số vào chỗ của mình) mà không đọc bảng `files_file`.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::storage::ByteStream;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// Thông tin một file đã tải lên.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileInfo {
    pub id: Uuid,
    /// Tên hiển thị (đã kiểm: không có `/`, `\`, ký tự điều khiển).
    pub name: String,
    pub content_type: String,
    /// Số byte.
    pub size: u64,
    /// SHA-256 của nội dung, hex chữ thường.
    pub sha256: String,
    pub created_at: Timestamp,
}

impl FileInfo {
    /// Dựng thông tin file (cho bản giả trong test của module khác).
    pub fn new(
        id: Uuid,
        name: impl Into<String>,
        content_type: impl Into<String>,
        size: u64,
        sha256: impl Into<String>,
        created_at: Timestamp,
    ) -> Self {
        FileInfo {
            id,
            name: name.into(),
            content_type: content_type.into(),
            size,
            sha256: sha256.into(),
            created_at,
        }
    }
}

/// Một file mở ra: thông tin và nội dung.
#[non_exhaustive]
pub struct OpenedFile {
    pub info: FileInfo,
    pub content: ByteStream,
}

impl OpenedFile {
    pub fn new(info: FileInfo, content: ByteStream) -> Self {
        OpenedFile { info, content }
    }
}

/// File người dùng đã tải lên. Module `files` công bố `dyn Files`.
#[async_trait::async_trait]
pub trait Files: Send + Sync {
    /// Thông tin file `id` của khách `tenant`, nếu `principal` được xem nó (chủ file còn
    /// `files.upload`, hay có `files.manage`). Không có hay không được xem: `files.file_not_found`
    /// (không lộ file của người khác có tồn tại).
    async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<FileInfo, Error>;

    /// Như [`find`](Self::find), kèm nội dung để đọc (chép, xử lý).
    async fn open(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<OpenedFile, Error>;
}
