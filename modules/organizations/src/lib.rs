//! Module `organizations`: cây tổ chức trong mỗi khách (bước 34, D-185) — trụ sở → vùng → cửa
//! hàng / kho / người bán…, loại nút do khách tự đặt, vòng đời của nút (chờ duyệt, hoạt động, tạm
//! khoá, lưu trữ: khoá một nút thì cả nhánh ngừng ghi), dời nút.
//!
//! Cấu trúc cây ở kernel (`autoxide_kernel::tenancy::OrgTree`, D-303) để module nào cũng lọc dữ
//! liệu của mình theo nhánh được; module này giữ phần nghĩa của nút và API quản lý cây.

pub mod api;
mod config;
mod entity;
mod events;
mod http;
mod migration;
mod permissions;
mod service;
mod settings;
mod tools;

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
use autoxide_kernel::tenancy::TenantSeed;
use utoipa_axum::router::OpenApiRouter;

use crate::config::OrganizationsConfig;
use crate::service::OrgService;

/// Module `organizations`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    OrganizationsModule::default()
}

#[derive(Default)]
struct OrganizationsModule {
    config: Option<OrganizationsConfig>,
    service: Option<Arc<OrgService>>,
}

impl Module for OrganizationsModule {
    fn name(&self) -> &'static str {
        "organizations"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(OrganizationsConfig::read(reader)?);
        Ok(())
    }

    fn org_settings(&self) -> Vec<autoxide_kernel::tenancy::OrgSetting> {
        settings::all()
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
            .ok_or_else(|| Error::internal("module `organizations` started before `configure`"))?;
        let service = Arc::new(OrgService::new(context.outbox()?, config, context.org_settings()));
        self.service = Some(service.clone());
        context.provide::<dyn api::OrgDirectory>(service)
    }

    fn seed_tenant(&self) -> Option<Arc<dyn TenantSeed>> {
        self.service.clone().map(|service| service as Arc<dyn TenantSeed>)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}
