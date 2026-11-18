//! Module `shipping` (bước 44): sổ địa chỉ người mua, phí vận chuyển khách tự đặt, hãng vận chuyển
//! cắm qua trait, vận đơn, đối soát tiền thu hộ (COD). `orders` báo giá và đọc sổ địa chỉ qua
//! `autoxide_shipping::api`, gửi yêu cầu giao qua event; `shipping` không phụ thuộc `orders` hay
//! `catalog`. Đặc tả: `docs/steps/44-shipping.md`. Hướng dẫn: `docs/SHIPPING.md`.

mod addresses;
pub mod api;
mod config;
mod entity;
mod errors;
mod http;
mod migration;
mod permissions;
mod privacy;
mod registry;
mod relocation;
mod service;

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
        let service = Arc::new(ShippingService::new(config, units));
        context.personal_data(service.clone())?;
        // `Arc<ShippingService>` tự đổi thành `Arc<dyn …>`: module khác chỉ thấy trait.
        context.provide::<dyn api::AddressBook>(service.clone())?;
        context.provide::<dyn api::CarrierRegistry>(registry)?;
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = self.service.clone() else {
            return Vec::new();
        };
        // Tài khoản bị xoá: xoá sổ địa chỉ (`privacy.rs`).
        vec![Subscription::new(
            autoxide_kernel::privacy::USER_DELETED,
            Arc::new(privacy::Erasure(service)),
        )]
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
