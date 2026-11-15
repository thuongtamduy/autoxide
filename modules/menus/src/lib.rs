//! Module `menus`: menu của website (thanh danh mục, bảng xổ lớn, menu chân trang, menu phụ) do
//! người quản trị dựng, mục trỏ tới địa chỉ hay tới đối tượng của bất kỳ module nào qua sổ đối
//! tượng của kernel (D-238, bước 68). Không nhầm với sổ điều hướng của bước 51
//! (`Module::navigation`): đó là menu của trang quản trị.
//!
//! Mọi bảng theo khách (bước 33): menu, cache menu công khai và lệnh nhập menu cũ đều của một
//! khách.
//!
//! Đăng ký SAU `media` (ảnh của ô gợi ý) và `content` (nhập menu cũ); đối tượng của module đăng
//! ký sau vẫn tra được, vì sổ đối tượng đọc lúc chạy.

mod client_config;
mod config;
mod entity;
mod events;
mod http;
mod items;
mod legacy;
mod migration;
mod permissions;
mod public;
mod service;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_content::api::LegacyMenus;
use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::task::Task;
use autoxide_media::api::Media;
use utoipa_axum::router::OpenApiRouter;

use crate::config::MenusConfig;
use crate::service::{Dependencies, MenuService};

/// Module `menus`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    MenusModule::default()
}

#[derive(Default)]
struct MenusModule {
    config: Option<MenusConfig>,
    /// Có sau `start`.
    service: Option<Arc<MenuService>>,
    /// Sổ database, có sau `start`: lệnh nhập menu cũ tra ngữ cảnh của khách được nêu.
    databases: Option<autoxide_kernel::tenancy::DatabaseRegistry>,
}

impl Module for MenusModule {
    fn name(&self) -> &'static str {
        "menus"
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

    fn client_config(
        &self,
    ) -> Result<Option<autoxide_kernel::module::ClientSection>, autoxide_kernel::error::Error> {
        self.config.as_ref().map_or(Ok(None), client_config::section)
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(MenusConfig::read(reader)?);
        Ok(())
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self
            .config
            .clone()
            .ok_or_else(|| Error::internal("module `menus` started before it was configured"))?;
        let dependencies = Dependencies {
            resources: context.resources(),
            media: context.require::<dyn Media>()?,
            // Không có `content` thì không có menu cũ để nhập: module vẫn chạy, chỉ lệnh nhập báo
            // không có nguồn.
            legacy: context.require::<dyn LegacyMenus>().ok(),
        };
        let service = Arc::new(MenuService::new(context.outbox()?, dependencies, config));
        self.service = Some(service);
        self.databases = Some(context.tenancy());
        Ok(())
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn tasks(&self) -> Vec<Task> {
        match (self.service.clone(), self.databases.clone()) {
            (Some(service), Some(databases)) => vec![legacy::task(service, databases)],
            _ => Vec::new(),
        }
    }
}
