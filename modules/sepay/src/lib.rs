//! Bộ nối **SePay** cho biến động số dư ngân hàng (bước 70 P7, D-20261121-vn-providers,
//! D-20261121-provider-accounts): cài `BankFeed` của kernel để `ledger` tự khớp thanh toán chuyển
//! khoản VietQR. Tiền vào thẳng tài khoản ngân hàng của khách; SePay chỉ báo biến động — core
//! không giữ tiền (D-187).
//!
//! **Hạng thử nghiệm**: written from the provider's public documentation; not yet verified against
//! the live service. Tài liệu đã dùng (đọc ngày 2026-11-21):
//!
//! - Webhook — payload, câu trả lời hợp lệ, chống trùng:
//!   <https://developer.sepay.vn/vi/sepay-webhooks/tich-hop-webhook>
//! - Webhook — xác thực (HMAC-SHA256, API Key): <https://developer.sepay.vn/vi/sepay-webhooks/xac-thuc>
//! - Webhook — bảo mật, lịch gửi lại: <https://developer.sepay.vn/vi/sepay-webhooks/bao-mat>,
//!   <https://developer.sepay.vn/vi/sepay-webhooks/xu-ly-loi>
//! - Đối soát bằng API: <https://developer.sepay.vn/vi/sepay-webhooks/doi-soat-giao-dich>
//! - SePay API v2 — xác thực, rate limit: <https://developer.sepay.vn/vi/sepay-api/v2/xac-thuc>
//! - SePay API v2 — danh sách giao dịch: <https://developer.sepay.vn/vi/sepay-api/v2/giao-dich/danh-sach>
//! - SePay API v2 — danh sách tài khoản ngân hàng:
//!   <https://developer.sepay.vn/vi/sepay-api/v2/tai-khoan-ngan-hang/danh-sach>
//!
//! Tài khoản SePay là của từng khách (cấu hình nhóm `connection`, xem [`settings`]); không có biến
//! môi trường nào cho thứ thuộc về khách. Đăng ký TRƯỚC `ledger`.

pub mod api;
mod config;
mod feed;
mod settings;
mod wire;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::payment::BankFeed;
use autoxide_kernel::provider::{ProviderAccounts, WebhookEndpoint};
use autoxide_kernel::tenancy::TenantSetting;

/// Kết nối của bộ nối (`<module>.connection.<loại>`).
pub(crate) const CONNECTION: &str = "sepay.connection.bank";

const WEBHOOK: WebhookEndpoint = WebhookEndpoint::new(
    CONNECTION,
    "/api/v1/ledger/bank-feeds/sepay",
    "Paste into my.sepay.vn > Webhooks > Add webhook > URL. Choose the JSON format and \
     HMAC-SHA256 (recommended) or API Key authentication, then store the same secret in \
     `sepay.connection.bank.webhook_secret` (or `…webhook_api_key`).",
);

#[derive(Default)]
struct SepayModule {
    config: config::SepayConfig,
}

/// Module `sepay` cho binary đăng ký (trước `ledger`).
pub fn module() -> impl Module {
    SepayModule::default()
}

impl Module for SepayModule {
    fn name(&self) -> &'static str {
        "sepay"
    }

    fn description(&self) -> &'static str {
        "Confirms VietQR bank transfers from SePay's bank transaction notifications, with one \
         SePay account per tenant (experimental: written from the provider's public \
         documentation; not yet verified against the live service)."
    }

    // Thử nghiệm (bước 70 P3): viết theo tài liệu công khai, chưa chạy với dịch vụ thật — chỉ chạy khi
    // được bật rõ trong `AUTOXIDE_MODULES`.
    fn stability(&self) -> autoxide_kernel::module::Stability {
        autoxide_kernel::module::Stability::Experimental
    }

    fn tenant_settings(&self) -> Vec<TenantSetting> {
        settings::all()
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = config::read(reader)?;
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        // `require` trả `Arc<ProviderAccounts>`; bản thân `ProviderAccounts` clone rẻ (bọc `Arc`).
        let accounts: ProviderAccounts = (*context.require::<ProviderAccounts>()?).clone();
        accounts.register_webhook(WEBHOOK);
        let feed = Arc::new(feed::SepayFeed::new(
            accounts,
            context.connection_status(),
            self.config.clone(),
        )?);
        context.connection_probe(feed.clone())?;
        context.contribute::<dyn BankFeed>(feed)
    }
}
