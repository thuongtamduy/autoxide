//! Module `shipping` (bước 44): sổ địa chỉ người mua, phí vận chuyển khách tự đặt, hãng vận chuyển
//! cắm qua trait, vận đơn, đối soát tiền thu hộ (COD). `orders` báo giá và đọc sổ địa chỉ qua
//! `autoxide_shipping::api`, gửi yêu cầu giao qua event; `shipping` không phụ thuộc `orders` hay
//! `catalog`. Đặc tả: `docs/steps/44-shipping.md`. Hướng dẫn: `docs/SHIPPING.md`.

mod addresses;
pub mod api;
mod booking;
mod calendars;
mod carriers;
mod cod;
mod config;
mod entity;
mod errors;
mod estimate;
mod eta;
mod events;
mod facts;
mod http;
mod jobs;
mod ledger;
mod limits;
mod listeners;
mod methods;
mod migration;
mod parcels;
mod permissions;
mod privacy;
mod quote;
mod registry;
mod relocation;
mod service;
mod settings;
mod shipments;
mod status;
mod tools;
mod webhooks;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::doctor::{Check, Finding, Probe};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Subscription;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::api::CarrierRegistry;
use crate::config::ShippingConfig;
use crate::registry::Registry;
use crate::service::ShippingService;

/// Module `shipping`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    ShippingModule::default()
}

#[derive(Default)]
struct ShippingModule {
    config: Option<ShippingConfig>,
    /// Có sau `start`.
    service: Option<Arc<ShippingService>>,
}

impl Module for ShippingModule {
    fn name(&self) -> &'static str {
        "shipping"
    }

    fn description(&self) -> &'static str {
        "Shipping: buyer address books, shipping rates set by each shop (zones by province, weight \
         tiers, free shipping, store pickup), carriers plugged in as connectors, shipments with \
         parcels and tracking, and cash-on-delivery reconciliation."
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

    fn features(&self) -> Vec<autoxide_kernel::entitlement::Feature> {
        service::features()
    }

    fn limits(&self) -> Vec<autoxide_kernel::entitlement::Limit> {
        vec![service::CARRIER_ACCOUNTS_LIMIT]
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, jobs::registrations)
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        jobs::schedules()
    }

    fn reason_lists(&self) -> Vec<autoxide_kernel::reasons::ReasonList> {
        use autoxide_kernel::reasons::{ReasonDefault, ReasonList};
        vec![ReasonList::new(
            cod::RESOLUTION_LIST,
            vec![
                ReasonDefault::new("carrier_corrected", "The carrier corrected its statement"),
                ReasonDefault::new("delivered_confirmed", "Delivery confirmed with the carrier"),
                ReasonDefault::new("returned_confirmed", "Return confirmed with the carrier"),
                ReasonDefault::new("written_off", "Written off: will not be collected"),
            ],
        )]
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
        // `address` đăng ký trước `shipping`; chưa cài thì không nhận địa chỉ Việt Nam (không
        // kiểm được xã thuộc tỉnh).
        let units = context.require::<dyn autoxide_address::api::AdministrativeUnits>().ok();
        let registry = Arc::new(Registry::default());
        if config.fake_carrier {
            registry.register(Arc::new(api::fake::FakeCarrier::new()))?;
        }
        let parts = service::Parts {
            outbox: context.outbox()?,
            jobs: context.jobs(),
            entitlements: context.entitlements(),
            databases: context.tenancy(),
            secret: (*context.require::<autoxide_kernel::secret::SecretKey>()?).clone(),
            providers: (*context.require::<autoxide_kernel::provider::ProviderAccounts>()?).clone(),
            carriers: registry.clone(),
            // `ledger` đăng ký trước `shipping`; chưa cài thì không ghi sổ COD.
            ledger: context.require::<dyn autoxide_ledger::api::Ledger>().ok(),
            reasons: context.reason_codes(),
            // `catalog` và `organizations` đăng ký trước `shipping`; thiếu thì không có ước tính công
            // khai / dùng giờ mở cửa mặc định.
            catalog: context.require::<dyn autoxide_catalog::api::Catalog>().ok(),
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            units,
        };
        let service = Arc::new(ShippingService::new(parts, config));
        context.personal_data(Arc::new(privacy::Erasure))?;
        // `Arc<ShippingService>` tự đổi thành `Arc<dyn …>`: module khác chỉ thấy trait.
        context.provide::<dyn api::AddressBook>(service.clone())?;
        context.provide::<dyn api::ShippingRates>(service.clone())?;
        context.provide::<dyn api::CarrierRegistry>(registry)?;
        // `support` đăng ký trước `shipping`; chưa cài thì không có trả lời tự động về vận đơn.
        if let Ok(facts) = context.require::<dyn autoxide_support::api::SupportFacts>() {
            facts.attach(facts::ShippingFacts::new(context.translations().clone()))?;
        }
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        // Yêu cầu giao / huỷ đơn của `orders` (có thể chưa cài: `optional`).
        let mut all = listeners::subscriptions(&service);
        // Tài khoản bị xoá: xoá sổ địa chỉ (`privacy.rs`).
        all.push(Subscription::new(
            autoxide_kernel::privacy::USER_DELETED,
            Arc::new(privacy::Erasure),
        ));
        all
    }

    // Chế độ "nghỉ" (bước 70 P9): module tắt vẫn xuất và xoá sổ địa chỉ của người mua.
    fn dormant(
        &self,
        context: &mut autoxide_kernel::module::DormantContext<'_>,
    ) -> Result<(), Error> {
        context.personal_data(Arc::new(privacy::Erasure))?;
        context.on_user_deleted(Arc::new(privacy::Erasure))
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

    fn checks(&self) -> Vec<Check> {
        let fake = self.config.as_ref().is_some_and(|config| config.fake_carrier);
        vec![Check::new(
            "fake_carrier",
            "The in-memory fake carrier is off (it must never run in production).",
            Arc::new(FakeCarrierProbe(fake)),
        )]
    }
}

/// `autoxide doctor`: cảnh báo khi hãng giả đang bật.
struct FakeCarrierProbe(bool);

#[async_trait::async_trait]
impl Probe for FakeCarrierProbe {
    async fn probe(&self) -> Finding {
        if self.0 {
            Finding::warning("the fake carrier `fake` is registered").with_hint(
                "Set AUTOXIDE_SHIPPING_FAKE_CARRIER=false outside development machines: shipments \
                 booked with it never reach a real carrier.",
            )
        } else {
            Finding::ok("the fake carrier is off")
        }
    }
}
