//! Gửi tin Zalo ZNS (bước 37, D-348): trait trung lập cho tin theo mẫu đã được Zalo duyệt. Bộ nối Zalo
//! là module riêng (cần khoá của Zalo Official Account) công bố `context.provide::<dyn ZnsSender>(…)`
//! và đăng ký TRƯỚC module `notifications`. Test dùng `FakeZnsSender` của testkit.

use serde_json::Value;

use crate::error::Error;
use crate::sms::PhoneNumber;
use crate::tenancy::TenantId;

/// Một tin ZNS cần gửi: mã mẫu đã duyệt và giá trị các tham số của mẫu.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ZnsMessage {
    pub tenant: TenantId,
    pub to: PhoneNumber,
    /// Mã mẫu ZNS đã được Zalo duyệt.
    pub template_id: String,
    /// Giá trị tham số của mẫu (`{"order_code": "…"}`).
    pub params: Value,
    /// Mã chống gửi trùng ở nhà cung cấp.
    pub reference: String,
}

impl ZnsMessage {
    pub fn new(
        tenant: TenantId,
        to: PhoneNumber,
        template_id: impl Into<String>,
        params: Value,
        reference: impl Into<String>,
    ) -> Self {
        ZnsMessage {
            tenant,
            to,
            template_id: template_id.into(),
            params,
            reference: reference.into(),
        }
    }
}

/// Một nhà cung cấp gửi tin ZNS.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ZnsSender: Send + Sync {
    fn provider(&self) -> &'static str;

    /// Trả mã tin ở nhà cung cấp. Lỗi `Unavailable` là tạm thời (thử lại); `Validation` là vĩnh viễn
    /// (số không dùng Zalo, mẫu chưa duyệt).
    async fn send(&self, message: ZnsMessage) -> Result<String, Error>;
}

/// `zns_disabled` (503): chưa cấu hình gửi Zalo ZNS.
pub fn zns_disabled() -> Error {
    Error::unavailable("zns_disabled", "Sending Zalo ZNS messages is not set up on this server.")
}
