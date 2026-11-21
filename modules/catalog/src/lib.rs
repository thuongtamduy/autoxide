//! Module `catalog` (bước 41): sản phẩm, biến thể, danh mục dạng cây, bảng giá, sàn giá — module
//! thương mại đầu tiên. `inventory`, `promotions`, `shipping`, `orders`, `channels`,
//! `marketplace` đọc sản phẩm và giá qua `autoxide_catalog::api`, không đọc bảng `catalog_…`.
//! Đặc tả: `docs/steps/41-catalog.md`.

pub mod api;
mod attributes;
mod bulk;
mod card;
mod categories;
mod collections;
mod config;
mod copywriting;
mod derived;
mod digital;
mod digital_access;
mod entity;
mod errors;
mod events;
mod facets;
mod floors;
mod from_images;
mod http;
mod listeners;
mod media_alts;
mod migration;
mod permissions;
mod pricing;
mod product_parts;
mod products;
mod public;
mod public_filter;
mod readiness;
mod recommend;
mod restrictions;
mod rounding;
mod rules_pack;
mod service;
mod similar;
mod snapshots;
mod storefront;
mod suggest;
mod synonyms;
mod tools;
mod tools_assist;
mod transfers;
mod variant_attributes;
mod variants;

mod relocation;
mod resources;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::entitlement::{Feature, Limit};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::CatalogConfig;
use crate::service::CatalogService;

/// Module `catalog`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    CatalogModule::default()
}

#[derive(Default)]
struct CatalogModule {
    config: Option<CatalogConfig>,
    /// Có sau `start`.
    service: Option<Arc<CatalogService>>,
}

impl Module for CatalogModule {
    fn name(&self) -> &'static str {
        "catalog"
    }

    fn description(&self) -> &'static str {
        "Products, variants, category tree, price lists with history and price floors: the \
         product catalog every commerce module reads."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
    }

    fn resource_events(&self) -> Vec<autoxide_kernel::resource::ResourceEvent> {
        resources::resource_events()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn features(&self) -> Vec<Feature> {
        service::features()
    }

    fn limits(&self) -> Vec<Limit> {
        vec![service::PRODUCTS_LIMIT]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(CatalogConfig::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().unwrap_or_default();
        // `media` và `tax` đăng ký trước `catalog`; chưa cài thì không gắn được ảnh / không áp
        // luật giá theo hồ sơ thuế.
        // `organizations` (cài đặt kế thừa theo nút) cũng vậy: chưa cài thì dùng mặc định.
        let service = Arc::new(CatalogService {
            outbox: context.outbox()?,
            entitlements: context.entitlements(),
            media: context.require::<dyn autoxide_media::api::Media>().ok(),
            tax: context.require::<dyn autoxide_tax::api::TaxEngine>().ok(),
            config,
            databases: context.tenancy(),
            jobs: context.jobs(),
            organizations: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            sale_prices: std::sync::OnceLock::new(),
            listing_gate: std::sync::OnceLock::new(),
            suggestions: context.suggest_sources(),
            embedder: context.require::<dyn autoxide_kernel::ai::Embedder>().ok(),
            storage: context.require::<dyn autoxide_kernel::storage::Storage>().ok(),
            text_model: context.require::<dyn autoxide_kernel::ai::TextModel>().ok(),
            files: context.require::<dyn autoxide_files::api::Files>().ok(),
            secrets: context.require::<autoxide_kernel::secret::SecretKey>()?,
        });
        // Bộ sưu tập trong sổ đối tượng: banner, menu trỏ tới (bước 55).
        context.resource_type(
            collections::resource::COLLECTION,
            Arc::new(collections::resource::CollectionResolver),
        )?;
        // Sản phẩm và danh mục trong sổ đối tượng: hỏi đáp (`comments`), menu (`menus`), nhãn
        // `seller` cho người quản lý sản phẩm (D-202, D-238).
        context.resource_type(
            resources::PRODUCT,
            Arc::new(resources::ProductResolver(service.clone())),
        )?;
        context.resource_type(
            resources::CATEGORY,
            Arc::new(resources::CategoryResolver(service.clone())),
        )?;
        context.badge_source(
            resources::PRODUCT,
            &[resources::SELLER],
            Arc::new(resources::SellerBadge),
        )?;
        context.suggest_source(Arc::new(suggest::ProductSuggestions(service.clone())))?;
        context.suggest_source(Arc::new(suggest::CategorySuggestions(service.clone())))?;
        context.usage_counter(&service::PRODUCTS_LIMIT, Arc::new(service::ProductCounter))?;
        self.service = Some(service.clone());
        // `Arc<CatalogService>` tự đổi thành `Arc<dyn Catalog>`: module khác chỉ thấy trait.
        context.provide::<dyn api::CatalogExtensions>(service.clone())?;
        context.provide::<dyn api::Catalog>(service)
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        // Quy tắc hàng cấm / có điều kiện: dữ liệu luật dùng chung mọi khách (D-198).
        vec!["catalog_restriction_rules"]
    }

    fn org_settings(&self) -> Vec<autoxide_kernel::tenancy::OrgSetting> {
        vec![bulk::SECOND_PERSON]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(
                    rules_pack::REFRESH_DOCUMENTS,
                    Arc::new(rules_pack::RefreshDocuments(service.clone())),
                ),
                JobRegistration::new(
                    derived::DERIVE_PRICES,
                    Arc::new(derived::Deriver(service.clone())),
                ),
                JobRegistration::new(bulk::APPLY_CHANGE, Arc::new(bulk::Applier(service.clone()))),
                JobRegistration::new(
                    storefront::REFRESH,
                    Arc::new(storefront::Refresher(service.clone())),
                ),
                JobRegistration::new(
                    storefront::SCHEDULED_PRICES,
                    Arc::new(storefront::ScheduledPrices(service.clone())),
                ),
                JobRegistration::new(similar::INDEX, Arc::new(similar::Indexer(service.clone()))),
                JobRegistration::new(
                    recommend::REFRESH_SALES,
                    Arc::new(recommend::RefreshSales(service.clone())),
                ),
                JobRegistration::new(
                    transfers::IMPORT,
                    Arc::new(transfers::Transfers(service.clone())),
                ),
                JobRegistration::new(
                    transfers::EXPORT,
                    Arc::new(transfers::Transfers(service.clone())),
                ),
                JobRegistration::new(
                    from_images::DRAFT_FROM_IMAGES,
                    Arc::new(from_images::Drafter(service.clone())),
                ),
                JobRegistration::new(transfers::CLEANUP, Arc::new(transfers::Cleanup(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        // Giấy tờ hết hạn theo ngày: tính lại `readiness` mỗi ngày.
        let day = std::time::Duration::from_secs(24 * 3600);
        // Giá hẹn giờ bắt đầu / hết hạn: dựng lại storefront mỗi phút.
        let minute = std::time::Duration::from_secs(60);
        vec![
            Schedule::new(rules_pack::REFRESH_DOCUMENTS, rules_pack::REFRESH_DOCUMENTS, day),
            Schedule::new(storefront::SCHEDULED_PRICES, storefront::SCHEDULED_PRICES, minute),
            // Số bán và mua cùng tính trong 90 ngày: bỏ dòng cũ mỗi ngày.
            Schedule::new(recommend::REFRESH_SALES, recommend::REFRESH_SALES, day),
            // File nhập / xuất giữ 7 ngày.
            Schedule::new(transfers::CLEANUP, transfers::CLEANUP, day),
        ]
    }

    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![rules_pack::task(service.clone()), storefront::task(service)]
        })
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        self.service.as_ref().map_or_else(Vec::new, listeners::subscriptions)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn body_limits(&self) -> Vec<autoxide_kernel::http::BodyLimit> {
        // File nhập CSV lớn hơn giới hạn chung của request JSON.
        vec![autoxide_kernel::http::BodyLimit::new(
            axum::http::Method::POST,
            "/imports",
            transfers::MAX_IMPORT_BYTES,
        )]
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
