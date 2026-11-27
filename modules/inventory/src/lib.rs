//! Module `inventory` (bước 42): tồn theo biến thể × kho (nút `warehouse` / `store`), sổ biến động
//! chỉ thêm, giữ chỗ không bao giờ bán quá tồn, chuyển kho, kiểm kê, nhập hàng. `orders` và module
//! khác dùng qua `autoxide_inventory::api`, không đọc bảng `inventory_…`. Hướng dẫn:
//! `docs/INVENTORY.md`; đặc tả: `docs/steps/42-inventory.md`.

mod alerts;
pub mod api;
mod config;
mod contract;
mod counts;
mod delivery;
mod entity;
mod errors;
mod events;
mod fulfil_plan;
mod http;
mod levels;
mod lots;
mod migration;
mod orders_events;
mod partial;
mod permissions;
mod preorders;
mod public;
mod purchasing;
mod receipts;
mod relocation;
mod reservations;
mod service;
mod settings;
mod stock;
mod tools;
mod transfers;
mod verify;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::doctor::Check;
use autoxide_kernel::entitlement::{Feature, Limit};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::reasons::ReasonList;
use autoxide_kernel::task::Task;
use autoxide_kernel::tenancy::{DatabaseRegistry, Relocation};
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

    // Module buộc phải có (`context.require::<…>()?` trong `start`); bật module này thì chúng được
    // bật kèm (bước 70 P1). Thứ dùng "nếu có" (`.ok()`) không khai ở đây.
    fn depends_on(&self) -> Vec<&'static str> {
        vec!["catalog"]
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

    fn relocation(&self) -> Relocation {
        relocation::relocation()
    }

    fn org_settings(&self) -> Vec<autoxide_kernel::tenancy::OrgSetting> {
        vec![lots::EXPIRING_DAYS]
    }

    fn reason_lists(&self) -> Vec<ReasonList> {
        levels::reason_lists()
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        vec![config::RESTOCK_CONFIRM_LINK]
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
            // `ledger` không bắt buộc: thiếu thì phiếu nhập không ghi sổ cái (`not_applicable`).
            ledger: context.require::<dyn autoxide_ledger::api::Ledger>().ok(),
            // `notifications` không bắt buộc: thiếu thì không có báo khi có hàng lại.
            notifier: context.require::<dyn autoxide_notifications::api::Notifier>().ok(),
            guard: context.require::<autoxide_kernel::antispam::SpamGuard>()?,
            links: context
                .require::<autoxide_kernel::tenancy::SiteLinks>()
                .map(|links| (*links).clone())
                .unwrap_or_default(),
        };
        if let Some(notifier) = &parts.notifier {
            alerts::declare(notifier.as_ref())?;
        }
        let service = Arc::new(InventoryService::new(parts, config));
        self.service = Some(service.clone());
        self.databases = Some(databases);
        // Đăng ký báo có hàng gắn tài khoản: vào bản xuất dữ liệu cá nhân (rà soát bảo mật 2026-11).
        context.personal_data(Arc::new(alerts::AlertPrivacy))?;
        // `Arc<InventoryService>` tự đổi thành `Arc<dyn Inventory>`: module khác chỉ thấy trait.
        context.provide::<dyn api::Inventory>(service)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = &self.service else { return Vec::new() };
        let mut subscriptions = contract::subscriptions(service);
        subscriptions.extend(orders_events::subscriptions(service));
        subscriptions.extend(alerts::subscriptions(service));
        // Tài khoản bị xoá: xoá đăng ký báo có hàng gắn tài khoản đó.
        subscriptions.push(Subscription::new(
            autoxide_kernel::privacy::USER_DELETED,
            Arc::new(alerts::AlertPrivacy),
        ));
        subscriptions
    }

    // Chế độ "nghỉ" (bước 70 P9): module tắt vẫn xuất / xoá đăng ký báo có hàng gắn tài khoản.
    fn dormant(
        &self,
        context: &mut autoxide_kernel::module::DormantContext<'_>,
    ) -> Result<(), Error> {
        context.personal_data(Arc::new(alerts::AlertPrivacy))?;
        context.on_user_deleted(Arc::new(alerts::AlertPrivacy))
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, contract::registrations)
    }

    fn schedules(&self) -> Vec<Schedule> {
        contract::schedules()
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
