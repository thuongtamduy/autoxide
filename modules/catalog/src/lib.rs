//! Module `catalog` (bước 41): sản phẩm, biến thể, danh mục dạng cây, bảng giá, sàn giá — module
//! thương mại đầu tiên. `inventory`, `promotions`, `shipping`, `orders`, `channels`,
//! `marketplace` đọc sản phẩm và giá qua `autoxide_catalog::api`, không đọc bảng `catalog_…`.
//! Đặc tả: `docs/steps/41-catalog.md`.

pub mod api;
mod categories;
mod config;
mod entity;
mod errors;
mod events;
mod http;
mod listeners;
mod migration;
mod permissions;
mod pricing;
mod products;
mod readiness;
mod service;
mod snapshots;
mod tools;
mod variants;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::entitlement::{Feature, Limit};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
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
        let media = context.require::<dyn autoxide_media::api::Media>().ok();
        let tax = context.require::<dyn autoxide_tax::api::TaxEngine>().ok();
        let service = Arc::new(CatalogService::new(
            context.outbox()?,
            context.entitlements(),
            media,
            tax,
            config,
        ));
        context.usage_counter(&service::PRODUCTS_LIMIT, Arc::new(service::ProductCounter))?;
        self.service = Some(service.clone());
        // `Arc<CatalogService>` tự đổi thành `Arc<dyn Catalog>`: module khác chỉ thấy trait.
        context.provide::<dyn api::Catalog>(service)
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
}
