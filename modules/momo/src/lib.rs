//! Module `momo`: bộ nối cổng thanh toán MoMo (bước 70 P7, D-20261121-vn-providers,
//! D-20261121-momo-adapter). Góp một `dyn PaymentGateway` cho `ledger`; tiền đi thẳng từ người mua
//! vào tài khoản MoMo của khách — hệ thống không giữ tiền (D-187).
//!
//! **Tài khoản là của từng khách**: Partner Code, Access Key, Secret Key là cấu hình theo khách
//! (`momo.connection.account.*`), không phải biến môi trường. IPN về URL riêng của từng khách
//! (`/api/v1/ledger/webhooks/momo/{hook}`) và được kiểm bằng Secret Key của đúng khách đó.
//!
//! **Viết theo tài liệu công khai của MoMo, chưa kiểm với dịch vụ thật.** API: "Payment Gateway"
//! bản v2 (đường dẫn `/v2/gateway/api/*`), tài liệu ở site v3. Nguồn, đọc ngày 2026-11-21:
//!
//! - Đọc trực tiếp — mã mẫu chính thức của MoMo, <https://github.com/momo-wallet/payment> (commit
//!   `d4746c5`): `python/MoMo.py` (tạo giao dịch và chuỗi ký), `php/PayMoMo/ipn_momo.php` (chuỗi ký
//!   của IPN), `php/PayMoMo/query_transaction.php` (truy vấn).
//! - Đọc GIÁN TIẾP qua trích đoạn của công cụ tìm kiếm, vì `developers.momo.vn` từ chối (HTTP 403)
//!   môi trường viết bộ nối này:
//!   - <https://developers.momo.vn/v3/docs/payment/api/wallet/onetime/> (tham số, giới hạn số tiền)
//!   - <https://developers.momo.vn/v3/docs/payment/api/result-handling/notification/> (IPN, trả 204)
//!   - <https://developers.momo.vn/v3/docs/payment/api/payment-api/query/>
//!   - <https://developers.momo.vn/v3/docs/payment/api/payment-api/refund/>
//!   - <https://developers.momo.vn/v3/docs/payment/api/result-handling/resultcode/>
//!   - <https://developers.momo.vn/v3/docs/payment/api/result-handling/idempotency/>
//!   - <https://developers.momo.vn/v3/docs/payment/onboarding/test-instructions/>
//!
//! Điểm tài liệu chưa nói rõ (cần đối chiếu khi có tài khoản thử): bảng trường trả về của truy
//! vấn; MoMo trả gì khi gửi lại cùng `requestId` / `orderId` (bộ nối coi mã 40, 41 là "đã có");
//! MoMo có gửi IPN cho hoàn tiền không (bộ nối coi là không); trang thanh toán một lần ghi trả
//! IPN bằng 200 kèm JSON còn trang thông báo ghi 204 (bộ nối theo trang thông báo).

mod account;
pub mod api;
mod gateway;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, Stability, StartContext};
use autoxide_kernel::payment::PaymentGateway;
use autoxide_kernel::provider::{ProviderAccounts, WebhookEndpoint};
use autoxide_kernel::tenancy::TenantSetting;

use crate::account::MomoConfig;
use crate::gateway::MomoGateway;

/// Module `momo`, để binary đăng ký trong `crates/app/src/modules.rs` — TRƯỚC `ledger` (bên dùng
/// `dyn PaymentGateway`).
pub fn module() -> impl Module {
    MomoModule::default()
}

#[derive(Default)]
struct MomoModule {
    /// Có sau `configure`.
    config: Option<MomoConfig>,
}

impl Module for MomoModule {
    fn name(&self) -> &'static str {
        api::PROVIDER
    }

    fn description(&self) -> &'static str {
        "MoMo payment gateway (Vietnam) with one MoMo account per tenant: payment links, payment \
         notifications, status checks and refunds. Written from the provider's public \
         documentation; not yet verified against the live service."
    }

    fn stability(&self) -> Stability {
        Stability::Experimental
    }

    fn tenant_settings(&self) -> Vec<TenantSetting> {
        vec![
            account::PARTNER_CODE,
            account::ACCESS_KEY,
            account::SECRET_KEY,
            account::ENVIRONMENT,
            account::RETURN_URL,
        ]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(account::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `momo` started before it was configured"))?;
        // `(*…).clone()`: `require` trả `Arc<ProviderAccounts>`; bản thân nó clone rẻ.
        let accounts = (*context.require::<ProviderAccounts>()?).clone();
        accounts.register_webhook(WebhookEndpoint::new(
            account::CONNECTION,
            gateway::WEBHOOK_PATH,
            "MoMo payment notifications (IPN). Nothing to paste at MoMo: the adapter sends this \
             address with every payment it creates. The server must be reachable at \
             AUTOXIDE_HTTP_PUBLIC_URL.",
        ));
        let gateway = Arc::new(MomoGateway::new(accounts, config, context.connection_status())?);
        // Thử tài khoản từ trang cài đặt (`POST /kernel/settings/{name}/test`).
        context.connection_probe(gateway.clone())?;
        context.contribute::<dyn PaymentGateway>(gateway)
    }
}
