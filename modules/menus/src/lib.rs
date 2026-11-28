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

mod blocks;
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
mod relocation;
mod service;
mod tools;

#[cfg(test)]
mod tests;
mod usages;

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
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("content", "Content", "file", 50)).item(
            NavItem::new("content.menus", "content", "Menus", 30)
                .permissions(&[crate::permissions::MENUS_READ]),
        )
    }

    fn name(&self) -> &'static str {
        "menus"
    }

    // Module buộc phải có (`context.require::<…>()?` trong `start`); bật module này thì chúng được
    // bật kèm (bước 70 P1). Thứ dùng "nếu có" (`.ok()`) không khai ở đây.
    fn depends_on(&self) -> Vec<&'static str> {
        vec!["media"]
    }

    fn description(&self) -> &'static str {
        "Website menus (top bar, mega menu, footer) whose items point to addresses or to \
         records of any module."
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
        let Some(config) = &self.config else { return Ok(None) };
        let section = client_config::section(config)?;
        // Ngôn ngữ là cấu hình theo khách: request xác định được khách thì lấy của khách.
        Ok(match &self.service {
            Some(service) => section.map(|section| {
                section.per_tenant(Arc::new(client_config::TenantLanguages(service.clone())))
            }),
            None => section,
        })
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        vec![config::TENANT_LOCALES, config::TENANT_DEFAULT_LOCALE]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(MenusConfig::read(reader)?);
        Ok(())
    }

    fn blocks(&self) -> Vec<autoxide_kernel::blocks::BlockType> {
        blocks::all()
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
            // Tuỳ chọn như `legacy`: không có `organizations` thì mọi website dùng cấu hình chung.
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            settings: context.require::<dyn autoxide_kernel::tenancy::TenantSettings>().ok(),
        };
        // Luật chéo lúc lưu (D-20261128-settings-one-system): ngôn ngữ mặc định đã đặt phải thuộc danh sách ngôn ngữ.
        let server_locales = config.locales.clone();
        context.require::<autoxide_kernel::tenancy::SettingSystem>()?.rule(
            autoxide_kernel::tenancy::SettingRule::new(
                &[config::TENANT_LOCALES.name(), config::TENANT_DEFAULT_LOCALE.name()],
                move |view| config::locales_fit(view, &server_locales),
            ),
        )?;
        let service = Arc::new(MenuService::new(context.outbox()?, dependencies, config));
        context.reference_source(Arc::new(usages::Menus));
        // Khối `menus.menu` của trang dựng từ khối (bước 67).
        context.block_data(blocks::MENU.name, Arc::new(blocks::MenuBlocks(service.clone())))?;
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

    // Tách / gộp khách (bước 38): chính sách ở `relocation.rs`.
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
