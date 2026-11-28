//! Bộ nối **SePay eInvoice** (bước 70 P7, D-20261121-vn-providers, D-20261121-provider-accounts,
//! D-20261121-sepay-einvoice): cài `EInvoiceProvider` của kernel để `invoices` gửi hoá đơn điện tử
//! tới cơ quan thuế qua SePay.
//!
//! **Hạng thử nghiệm**: written from the provider's public documentation; not yet verified against
//! the live service. Tài liệu đã dùng (đọc ngày 2026-11-21):
//!
//! - Tổng quan, luồng xuất hoá đơn: <https://developer.sepay.vn/vi/einvoice-api/v1/tong-quan>
//! - Token (Basic Authentication, sống 24 giờ): <https://developer.sepay.vn/vi/einvoice-api/v1/tao-token>
//! - Tài khoản nhà cung cấp, mẫu và ký hiệu:
//!   <https://developer.sepay.vn/vi/einvoice-api/v1/danh-sach-tai-khoan>,
//!   <https://developer.sepay.vn/vi/einvoice-api/v1/chi-tiet-tai-khoan>
//! - Xuất hoá đơn: <https://developer.sepay.vn/vi/einvoice-api/v1/xuat-hoa-don-dien-tu>
//! - Hoá đơn thay thế, điều chỉnh: <https://developer.sepay.vn/vi/einvoice-api/v1/xuat-hoa-don-thay-the>,
//!   <https://developer.sepay.vn/vi/einvoice-api/v1/xuat-hoa-don-dieu-chinh>
//! - Theo dõi trạng thái: <https://developer.sepay.vn/vi/einvoice-api/v1/theo-doi-trang-thai-xuat-hoa-don>
//! - Chi tiết hoá đơn: <https://developer.sepay.vn/vi/einvoice-api/v1/chi-tiet-hoa-don>
//! - Tải hoá đơn: <https://developer.sepay.vn/vi/einvoice-api/v1/tai-hoa-don>
//! - Xử lý lỗi, trước khi gửi lại: <https://developer.sepay.vn/vi/einvoice-api/v1/xu-ly-loi>
//!
//! Tài khoản SePay eInvoice là của từng khách (cấu hình nhóm `connection`, xem [`settings`]). Đăng
//! ký TRƯỚC `invoices`.

pub mod api;
mod client;
mod config;
mod provider;
mod settings;
mod wire;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::connections::ConnectionProbe;
use autoxide_kernel::einvoice::EInvoiceProvider;
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Manifest, Module, Stability, StartContext};
use autoxide_kernel::provider::ProviderAccounts;

/// Kết nối của bộ nối (`<module>.connection.<loại>`).
pub(crate) const CONNECTION: &str = "einvoice_sepay.connection.api";

#[derive(Default)]
struct SepayEInvoiceModule {
    config: config::EInvoiceConfig,
}

/// Module `einvoice_sepay` cho binary đăng ký (trước `invoices`).
pub fn module() -> impl Module {
    SepayEInvoiceModule::default()
}

// Hợp đồng module (D-20261126-module-manifest): khai tĩnh trong `manifest`, đăng ký thứ cần service trong `start`.
impl Module for SepayEInvoiceModule {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            "einvoice_sepay",
            "Sends e-invoices to the Vietnamese tax authority through SePay eInvoice, with one SePay \
             eInvoice account per tenant (experimental: written from the provider's public \
             documentation; not yet verified against the live service).",
        )
        // Thử nghiệm (bước 70 P3): viết theo tài liệu công khai, chưa chạy với dịch vụ thật — chỉ chạy khi
        // được bật rõ trong `AUTOXIDE_MODULES`.
        .stability(Stability::Experimental)
        .tenant_settings(settings::all())
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = config::read(reader)?;
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        // `require` trả `Arc<ProviderAccounts>`; bản thân `ProviderAccounts` clone rẻ (bọc `Arc`).
        let accounts: ProviderAccounts = (*context.require::<ProviderAccounts>()?).clone();
        let client = client::Client::new(accounts, context.kernel()?, self.config.clone())?;
        let provider = Arc::new(provider::SepayEInvoice::new(client));
        context.contribute::<dyn ConnectionProbe>(provider.clone())?;
        context.contribute::<dyn EInvoiceProvider>(provider)
    }
}
