//! Module `promotions` (bước 43): chương trình khuyến mại là luật — giảm tự động, coupon, voucher
//! trong ví; mua X tặng Y, flash sale, bậc thang, combo, mua kèm, chọn quà; cộng dồn theo ba lớp
//! `product` / `order` / `shipping`; giữ → xác nhận → trả lượt dùng cho `orders`; hạn mức pháp
//! luật là dữ liệu có ngày hiệu lực. `orders` (bước 45) gọi `autoxide_promotions::api`.
//! Đặc tả: `docs/steps/43-promotions.md`.

pub mod api;
mod codes;
mod compiled;
mod compliance;
mod config;
mod engine;
mod entity;
mod evaluate;
mod events;
mod http;
mod jobs;
mod legal;
mod migration;
mod model;
mod notices;
mod permissions;
mod programs;
mod provider;
mod public;
mod reasons;
mod redemption;
mod relocation;
mod service;
mod settings;
mod tools;
mod vouchers;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};

use crate::config::PromotionsConfig;
use crate::service::PromotionsService;

/// Module `promotions`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    PromotionsModule::default()
}

#[derive(Default)]
struct PromotionsModule {
    config: Option<PromotionsConfig>,
    /// Có sau `start`.
    service: Option<Arc<PromotionsService>>,
}

impl Module for PromotionsModule {
    fn name(&self) -> &'static str {
        "promotions"
    }

    fn description(&self) -> &'static str {
        "Promotions as rules: automatic discounts, coupons and vouchers, flash sales, tiers, \
         bundles and gifts, stacking in three classes, usage holds for orders and legal limits as \
         dated data."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        // Dữ liệu luật dùng chung mọi khách (D-198); khai bằng chữ để `cargo xtask check` đọc được.
        vec!["promotions_legal_packs", "promotions_legal_limits", "promotions_notice_rules"]
    }

    fn org_settings(&self) -> Vec<autoxide_kernel::tenancy::OrgSetting> {
        settings::all()
    }

    fn limits(&self) -> Vec<autoxide_kernel::entitlement::Limit> {
        vec![programs::transitions::ACTIVE_PROMOTIONS]
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
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
        // `catalog` đăng ký trước `promotions` (bắt buộc); `organizations` có thì đọc cài đặt kế
        // thừa và múi giờ cửa hàng.
        let catalog = context.require::<dyn autoxide_catalog::api::Catalog>()?;
        let orgs = context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok();
        let service = Arc::new(PromotionsService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            entitlements: context.entitlements(),
            catalog,
            orgs,
            translations: context.translations().clone(),
            config,
            legal_cache: legal::LegalCache::default(),
            compiled: compiled::CompiledCache::default(),
            jobs: context.jobs(),
        });
        context.personal_data(service.clone())?;
        self.service = Some(service.clone());
        // `catalog` không gọi thẳng `dyn Promotions` được (promotions phụ thuộc catalog — gọi ngược
        // là vòng phụ thuộc), nên promotions cắm nguồn giá giảm tự động vào catalog. Catalog thật
        // luôn có điểm cắm; bản giả trong test có thể không có thì bỏ qua.
        if let Ok(extensions) = context.require::<dyn autoxide_catalog::api::CatalogExtensions>() {
            extensions.set_sale_price_source(Arc::new(provider::SalePrices(service.clone())));
        }
        // `Arc<Provider>` tự đổi thành `Arc<dyn Promotions>`: module khác chỉ thấy trait.
        context.provide::<dyn api::Promotions>(Arc::new(provider::Provider(service)))
    }

    fn routes(&self) -> utoipa_axum::router::OpenApiRouter {
        self.service.clone().map_or_else(utoipa_axum::router::OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<autoxide_kernel::mcp::Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, jobs::registrations)
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        jobs::schedules()
    }

    fn subscriptions(&self) -> Vec<autoxide_kernel::event::Subscription> {
        vec![autoxide_kernel::event::Subscription::new(
            autoxide_kernel::privacy::USER_DELETED,
            Arc::new(vouchers::ErasedUsers),
        )]
    }

    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        self.service.clone().map_or_else(Vec::new, |service| vec![legal::task(service)])
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![autoxide_kernel::doctor::Check::new(
                "promotion_limits",
                "Promotion law limits are loaded and none ends soon without one after it.",
                Arc::new(legal::LegalCheck(service)),
            )]
        })
    }
}
