//! Module `esms`: bộ nối gửi SMS qua eSMS (Việt Nam) — cài `SmsSender` của kernel (bước 70 P7,
//! D-20261121-vn-providers, D-20261121-esms-adapter).
//!
//! **Hạng thử nghiệm: viết theo tài liệu công khai của eSMS, chưa kiểm với dịch vụ thật.**
//!
//! Tài liệu đã dùng (đọc ngày 2026-11-21):
//! - Cơ chế, xác thực bằng ApiKey + SecretKey: <https://developers.esms.vn/readme/co-che-hoat-dong>,
//!   <https://developers.esms.vn/readme/lay-thong-tin-apikey-va-secretkey>
//! - Gửi tin OTP / chăm sóc khách hàng (`SendMultipleMessage_V4_post_json`):
//!   <https://developers.esms.vn/esms-api/ham-gui-tin/tin-nhan-sms-otp-cskh>
//! - Tin quảng cáo (KHÔNG dùng; ít nhất 30 số mỗi request):
//!   <https://developers.esms.vn/esms-api/ham-gui-tin/tin-sms-quang-cao>
//! - Lấy số dư (`GetBalance_json`, dùng để thử kết nối):
//!   <https://developers.esms.vn/esms-api/ham-truy-xuat-va-dang-ky/ham-lay-so-du-tai-khoan>
//! - Bảng mã lỗi: <https://developers.esms.vn/esms-api/bang-ma-loi>
//! - Môi trường test (trường `Sandbox`): <https://developers.esms.vn/esms-api/moi-truong-test-sandbox>
//! - Callback trạng thái (KHÔNG dùng; GET không có chữ ký):
//!   <https://developers.esms.vn/esms-api/callback-url>
//!
//! Cách chạy:
//! - Tài khoản eSMS là của TỪNG KHÁCH (`esms.connection.sms.*`, xem `account`); module công bố một
//!   `dyn SmsSender` và mỗi lần gửi lấy tài khoản của đúng khách trong `SmsMessage::tenant`. Khách
//!   chưa cấu hình nhận lỗi `provider.not_configured`.
//! - Chỉ gửi mã một lần và tin giao dịch (tin chăm sóc khách hàng, `SmsType` 2). Tin quảng cáo bị
//!   từ chối (`esms.marketing_unsupported`).
//! - Module không có bảng, không route, không tool MCP.

mod account;
pub mod api;
mod client;
mod config;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::connections::ConnectionProbe;
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Manifest, Module, Stability, StartContext};
use autoxide_kernel::provider::ProviderAccounts;
use autoxide_kernel::sms::SmsSender;

use crate::config::EsmsConfig;

/// Module `esms`, để binary đăng ký trong `crates/app/src/modules.rs` — TRƯỚC các module gửi SMS
/// (`compliance`, `notifications`, `bookings`, `orders`).
pub fn module() -> impl Module {
    EsmsModule::default()
}

#[derive(Default)]
struct EsmsModule {
    /// Có sau `configure`.
    config: Option<EsmsConfig>,
}

// Hợp đồng module (D-20261126-module-manifest): khai tĩnh trong `manifest`, đăng ký thứ cần service trong `start`.
impl Module for EsmsModule {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            "esms",
            "SMS through eSMS (Vietnam): one-time codes and customer-care messages under each \
             tenant's own eSMS account and brandname. Written from the provider's public \
             documentation; not yet verified against the live service.",
        )
        .stability(Stability::Experimental)
        .tenant_settings([
            account::API_KEY,
            account::SECRET_KEY,
            account::BRANDNAME,
            account::ENVIRONMENT,
        ])
        .translation("vi", include_str!("../locales/vi.json"))
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(EsmsConfig::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `esms` started before it was configured"))?;
        // `ProviderAccounts` do kernel công bố (không phải module nào), nên `depends_on` rỗng.
        // `require` trả `Arc<ProviderAccounts>`; bản thân nó clone rẻ nên lấy một bản riêng.
        let accounts = (*context.require::<ProviderAccounts>()?).clone();
        let esms = Arc::new(client::Esms::new(accounts, config.api_url)?);
        context.contribute::<dyn ConnectionProbe>(esms.clone())?;
        context.provide::<dyn SmsSender>(esms)
    }
}
