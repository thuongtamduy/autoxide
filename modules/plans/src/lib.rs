//! Module `plans` (bước 35, D-185, D-323): bán phần mềm theo gói mà không sửa code. Gói là dữ liệu
//! (module + tính năng + hạn mức + giá theo kỳ), có gói thêm và ghi đè riêng cho từng khách; module
//! công bố quyền dùng gộp lại của mỗi khách cho kernel (`dyn Entitlements`), kernel cưỡng chế.
//!
//! Không có gói nào (hay không có gói mặc định) thì khách không thuê bao không bị giới hạn: cài
//! module này vào bản một khách không đổi gì.

pub mod api;
mod billing_profile;
mod config;
mod entity;
mod events;
mod http;
mod http_lifecycle;
mod jobs;
mod lifecycle;
mod merge;
mod migration;
mod permissions;
mod provider;
mod service;
mod tools;
mod usage;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::PlansConfig;
use crate::service::{Cache, PlansService};

pub fn module() -> impl Module {
    PlansModule::default()
}

#[derive(Default)]
struct PlansModule {
    config: Option<PlansConfig>,
    service: Option<Arc<PlansService>>,
}

impl Module for PlansModule {
    fn name(&self) -> &'static str {
        "plans"
    }

    // Gói không tự tắt được chính module quản lý gói.
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        vec![
            "plans_plan",
            "plans_plan_module",
            "plans_plan_feature",
            "plans_plan_limit",
            "plans_price",
            "plans_subscription",
            "plans_subscription_addon",
            "plans_override",
            "plans_settings",
            "plans_usage",
            "plans_reservation",
            "plans_usage_record",
            "plans_billing_period",
            "plans_billing_profile",
        ]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(PlansConfig::read(reader)?);
        Ok(())
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

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `plans` started before `configure`"))?;
        let service = Arc::new(PlansService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            entitlements: context.entitlements(),
            cache: Cache::new(config.cache),
            hold: config.hold,
            tenants: context.require::<autoxide_kernel::tenancy::TenantAdmin>().ok(),
            mailer: context.require::<dyn autoxide_kernel::mail::Mailer>().ok(),
            translations: context.translations().clone(),
            reminder_days: config.reminder_days.clone(),
            exports: context.require::<autoxide_kernel::tenancy::TenantExports>().ok(),
            units: context.require::<dyn autoxide_address::api::AdministrativeUnits>().ok(),
        });
        self.service = Some(service.clone());
        context.provide::<dyn autoxide_kernel::entitlement::Entitlements>(Arc::new(
            provider::Provider(service),
        ))
    }

    fn jobs(&self) -> Vec<autoxide_kernel::job::JobRegistration> {
        self.service.as_ref().map_or_else(Vec::new, jobs::registrations)
    }

    fn schedules(&self) -> Vec<autoxide_kernel::job::Schedule> {
        jobs::schedules()
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}
