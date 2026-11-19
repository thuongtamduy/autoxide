//! Module `orders` (bước 45): giỏ hàng cho người mua và vãng lai, đặt hàng là chuỗi giữ chỗ có hạn
//! (tồn kho, lượt khuyến mại) rồi tạo đơn có số tuần tự, thanh toán qua đối tác có giấy phép theo
//! chế độ dòng tiền của khách, máy trạng thái của đơn, đổi trả, hoàn tiền, hoá đơn điện tử, bán tại
//! quầy, đánh giá sau mua. Gọi `catalog`, `inventory`, `promotions`, `shipping`, `tax`, `ledger` qua
//! `api` của chúng; core không bao giờ tự giữ tiền của người mua (D-187).
//! Đặc tả: `docs/steps/45-orders.md`.

mod abandoned;
mod accounting;
mod actions;
pub mod api;
mod carts;
mod checkout;
mod config;
mod entity;
mod errors;
mod events;
mod extensions;
mod holds;
mod http;
mod jobs;
mod lifecycle;
mod migration;
mod orders_read;
mod payments;
mod permissions;
mod pickups;
mod policy;
mod pricing;
mod reasons;
mod refunds;
mod relocation;
mod return_refund;
mod returns;
mod returns_buyer;
mod service;
mod shipping_events;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::OrdersConfig;
use crate::extensions::Extensions;
use crate::service::OrdersService;

/// Module `orders`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    OrdersModule::default()
}

#[derive(Default)]
struct OrdersModule {
    /// Điểm cắm cho module đăng ký sau (bước 46–49); có từ lúc tạo để `start` trao ra ngoài.
    extensions: Arc<Extensions>,
    config: Option<OrdersConfig>,
    /// Có sau `start`.
    service: Option<Arc<OrdersService>>,
}

impl Module for OrdersModule {
    fn name(&self) -> &'static str {
        "orders"
    }

    fn description(&self) -> &'static str {
        "Orders: carts for buyers and guests, checkout with time-limited holds of stock and \
         promotions, orders with sequential numbers and a lifecycle, payments through licensed \
         partners, returns and refunds, electronic invoices, point of sale, product reviews."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<autoxide_kernel::event::EventInfo> {
        events::all()
    }

    fn org_settings(&self) -> Vec<autoxide_kernel::tenancy::OrgSetting> {
        policy::all()
    }

    fn reason_lists(&self) -> Vec<autoxide_kernel::reasons::ReasonList> {
        reasons::lists()
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
        // `catalog` bắt buộc (giá, biến thể); module khác chưa cài thì bước đó bỏ qua: không giữ
        // tồn, không khuyến mại, không phí giao, thuế 0.
        let sources = pricing::Sources {
            catalog: context.require::<dyn autoxide_catalog::api::Catalog>()?,
            inventory: context.require::<dyn autoxide_inventory::api::Inventory>().ok(),
            promotions: context.require::<dyn autoxide_promotions::api::Promotions>().ok(),
            shipping: context.require::<dyn autoxide_shipping::api::ShippingRates>().ok(),
            tax: context.require::<dyn autoxide_tax::api::TaxEngine>().ok(),
        };
        let parts = service::Parts {
            outbox: context.outbox()?,
            sources,
            address_book: context.require::<dyn autoxide_shipping::api::AddressBook>().ok(),
            units: context.require::<dyn autoxide_address::api::AdministrativeUnits>().ok(),
            extensions: self.extensions.clone(),
            databases: context.tenancy(),
            // `ledger` đăng ký trước `orders`; chưa cài thì chỉ nhận COD.
            ledger: context.require::<dyn autoxide_ledger::api::Ledger>().ok(),
            secret: (*context.require::<autoxide_kernel::secret::SecretKey>()?).clone(),
            // Chính sách theo nút đọc từ cài đặt kế thừa; chưa cài `organizations`: mặc định.
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            reasons: context.reason_codes(),
            // Ảnh kèm yêu cầu trả: hỏi `media` ai đã tải lên (chưa cài: không nhận ảnh).
            media: context.require::<dyn autoxide_media::api::Media>().ok(),
        };
        let service = Arc::new(OrdersService::new(parts, config));
        // `Arc<OrdersService>` tự đổi thành `Arc<dyn Orders>`: module khác chỉ thấy trait.
        context.provide::<dyn api::Orders>(service.clone())?;
        context.provide::<dyn api::OrderHolds>(service.clone())?;
        context.provide::<dyn api::OrderRefunds>(service.clone())?;
        self.service = Some(service);
        // `Arc<Extensions>` tự đổi thành `Arc<dyn OrderExtensions>`: module khác chỉ thấy trait.
        context.provide::<dyn api::OrderExtensions>(self.extensions.clone())?;
        Ok(())
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, jobs::registrations)
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        jobs::schedules()
    }

    fn subscriptions(&self) -> Vec<autoxide_kernel::event::Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        let mut all = holds::subscriptions(&service);
        all.extend(accounting::subscriptions(&service));
        all.extend(shipping_events::subscriptions(&service));
        // Thanh toán xong ở `ledger` (có thể chưa cài: `optional`).
        all.push(autoxide_kernel::event::Subscription::optional(
            payments::LEDGER_PAYMENT_SUCCEEDED,
            Arc::new(payments::LedgerEvents(service.clone())),
        ));
        // Cổng hoàn xong sau (lượt hoàn `pending`).
        all.push(autoxide_kernel::event::Subscription::optional(
            refunds::LEDGER_PAYMENT_REFUNDED,
            Arc::new(refunds::LedgerRefunds(service)),
        ));
        all
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
