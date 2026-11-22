//! Bộ nối hãng vận chuyển Giao Hàng Nhanh (GHN) cho module `shipping` (bước 70 P7,
//! D-20261121-vn-providers, D-20261121-ghn-carrier). Hạng thử nghiệm: viết theo tài liệu công khai
//! của GHN, CHƯA chạy với dịch vụ thật.
//!
//! Tài liệu đã dùng (https://developer.ghn.vn — `api.ghn.vn/home/docs` chuyển hướng về đây), đọc
//! ngày 2026-11-21:
//! - Tổng quan, môi trường: https://developer.ghn.vn/en/docs/getting-started/overview
//! - Token, ShopId: https://developer.ghn.vn/en/docs/token/get-token
//! - Cửa hàng của token: https://developer.ghn.vn/en/docs/shop/get-shop
//! - Tạo đơn: https://developer.ghn.vn/en/docs/order/create
//! - Xem trước đơn (phí, ngày giao dự kiến): https://developer.ghn.vn/en/docs/order/preview
//! - Tính phí, thời gian giao, dịch vụ (chỉ nhận mã quận cũ — không dùng):
//!   https://developer.ghn.vn/en/docs/order/calculate-fee,
//!   https://developer.ghn.vn/en/docs/order/leadtime,
//!   https://developer.ghn.vn/en/docs/master-data/get-service
//! - Chi tiết đơn: https://developer.ghn.vn/en/docs/order/info và
//!   https://developer.ghn.vn/en/docs/order/info-by-client-code
//! - Huỷ đơn: https://developer.ghn.vn/en/docs/order/cancel
//! - In vận đơn: https://developer.ghn.vn/en/docs/order/print
//! - Webhook trạng thái: https://developer.ghn.vn/en/docs/webhook/callback-order-status
//! - Trạng thái đơn, mã lý do: https://developer.ghn.vn/en/docs/master-data/order-status,
//!   https://developer.ghn.vn/en/docs/master-data/reason-code
//! - Địa chỉ hai cấp (sau 01/07/2025): https://developer.ghn.vn/en/docs/master-data/get-province-new,
//!   https://developer.ghn.vn/en/docs/master-data/get-ward-new
//!
//! Tài khoản GHN là của từng khách: nằm trong tài khoản hãng của `shipping`
//! (`POST /api/v1/shipping/carrier-accounts`, `credentials.token` mã hoá, `settings.shop_id`), không
//! có biến môi trường nào mang token. Bộ nối chỉ dùng phần gọi mạng của
//! `autoxide_kernel::provider` (hạn giờ, không theo chuyển hướng, log không có secret).
//! Hướng dẫn cho khách: `docs/guides/providers/ghn.md`.

mod account;
pub mod api;
mod carrier;
mod client;
mod config;
mod errors;
mod status;
mod webhook;

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, Stability, StartContext};
use autoxide_shipping::api::CarrierRegistry;

use crate::config::GhnConfig;

/// Module `ghn`, để binary đăng ký trong `crates/app/src/modules.rs` — SAU `shipping`.
pub fn module() -> impl Module {
    GhnModule::default()
}

#[derive(Default)]
struct GhnModule {
    config: Option<GhnConfig>,
}

impl Module for GhnModule {
    fn name(&self) -> &'static str {
        api::PROVIDER
    }

    fn description(&self) -> &'static str {
        "Giao Hang Nhanh (GHN) carrier connector for shipping: quotes, shipments, labels, \
         cancellation, tracking and status webhooks, with each tenant's own GHN account. Written \
         from the provider's public documentation; not yet verified against the live service."
    }

    fn stability(&self) -> Stability {
        Stability::Experimental
    }

    fn depends_on(&self) -> Vec<&'static str> {
        vec!["shipping"]
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(config::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().unwrap_or_default();
        // `shipping` đăng ký trước và công bố sổ hãng; bộ nối tự ghi tên mình vào đó.
        let registry = context.require::<dyn CarrierRegistry>()?;
        let carrier = carrier::GhnCarrier::new(client::Client::new(config)?);
        registry.register(Arc::new(carrier))
    }
}
