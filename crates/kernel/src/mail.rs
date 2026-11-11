//! Gửi mail: trait chung để module nào cũng gửi được mà không phụ thuộc cách gửi.
//!
//! Một module (`mail`) công bố `dyn Mailer`; module cần gửi mail lấy nó bằng
//! `context.require::<dyn Mailer>()` (đăng ký SAU module `mail`). `send` chỉ đưa mail vào
//! hàng đợi rồi trả về ngay: việc gửi chạy nền, lỗi thì thử lại (D-061).

use crate::error::Error;

/// Một mail cần gửi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    /// Địa chỉ người nhận (`ada@example.com` hoặc `Ada <ada@example.com>`).
    pub to: String,
    pub subject: String,
    /// Nội dung dạng chữ (luôn có: mọi ứng dụng mail đọc được).
    pub text: String,
    /// Nội dung HTML, nếu có.
    pub html: Option<String>,
}

/// Bên gửi mail của chương trình.
#[async_trait::async_trait]
pub trait Mailer: Send + Sync {
    /// Đưa `mail` vào hàng đợi gửi. Lỗi khi địa chỉ người nhận sai dạng
    /// (`validation_failed`, trường `to`) hay không ghi được hàng đợi.
    async fn send(&self, mail: Mail) -> Result<(), Error>;
}
