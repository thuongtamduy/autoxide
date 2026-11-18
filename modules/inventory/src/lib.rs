//! Module `inventory` (bước 42): tồn theo biến thể × kho (nút `warehouse` / `store`), sổ biến động
//! chỉ thêm, giữ chỗ không bao giờ bán quá tồn, chuyển kho, kiểm kê, nhập hàng. `orders` và module
//! khác dùng qua `autoxide_inventory::api`, không đọc bảng `inventory_…`. Hướng dẫn:
//! `docs/INVENTORY.md`; đặc tả: `docs/steps/42-inventory.md`.

pub mod api;
mod config;
mod entity;
mod errors;
mod events;
mod http;
mod levels;
mod migration;
mod permissions;
mod service;
mod stock;
mod tools;
mod verify;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::doctor::Check;
use autoxide_kernel::entitlement::{Feature, Limit};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::reasons::ReasonList;
use autoxide_kernel::task::Task;
use autoxide_kernel::tenancy::DatabaseRegistry;
use utoipa_axum::router::OpenApiRouter;

use crate::config::InventoryConfig;
use crate::service::{InventoryService, Parts};

/// Module `inventory`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    InventoryModule::default()
}

#[derive(Default)]
struct InventoryModule {
    config: Option<InventoryConfig>,
    /// Có sau `start`.
    service: Option<Arc<InventoryService>>,
    databases: Option<DatabaseRegistry>,
}

impl Module for InventoryModule {
    fn name(&self) -> &'static str {
        "inventory"
    }

    fn description(&self) -> &'static str {
        "Stock per variant and warehouse with an append-only movement ledger, reservations that \
         never oversell, transfers, stock counts, lots, serials and purchasing."
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
        vec![service::WAREHOUSES_LIMIT]
    }

    fn reason_lists(&self) -> Vec<ReasonList> {
        levels::reason_lists()
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(config::read(reader)?);
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().unwrap_or_default();
        let databases = context.tenancy();
        let parts = Parts {
            outbox: context.outbox()?,
            entitlements: context.entitlements(),
            databases: databases.clone(),
            // `catalog` đăng ký trước `inventory` và là bắt buộc: tồn là của biến thể.
            catalog: context.require::<dyn autoxide_catalog::api::Catalog>()?,
            // `organizations` không bắt buộc: thiếu thì không kiểm loại nút kho.
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            reasons: context.reason_codes(),
        };
        let service = Arc::new(InventoryService::new(parts, config));
        self.service = Some(service);
        self.databases = Some(databases);
        Ok(())
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn tasks(&self) -> Vec<Task> {
        match (&self.service, &self.databases) {
            (Some(service), Some(databases)) => {
                vec![verify::task(service.clone(), databases.clone())]
            }
            _ => Vec::new(),
        }
    }

    fn checks(&self) -> Vec<Check> {
        match (&self.service, &self.databases) {
            (Some(service), Some(databases)) => {
                vec![verify::check(service.clone(), databases.clone())]
            }
            _ => Vec::new(),
        }
    }
}
