//! Module mẫu `notes`: ghi chú cá nhân (D-069).
//!
//! Đây là KHUÔN cho module về sau, dùng đủ mọi phần của hợp đồng module: migration,
//! permission, service, REST + OpenAPI, event (vào audit log), tool MCP, bản dịch, và
//! trait công bố cho module khác. Không đăng ký sẵn trong binary — core trung lập với
//! nghiệp vụ. Cách tạo module mới từ khuôn này: `docs/MODULE_GUIDE.md`.

pub mod api;
mod entity;
mod events;
mod http;
mod migration;
mod permissions;
mod service;
mod tools;

#[cfg(test)]
mod branch_tests;
#[cfg(test)]
mod tenancy_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::service::NoteService;

/// Module `notes`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    NotesModule::default()
}

#[derive(Default)]
struct NotesModule {
    /// Có sau `start`.
    service: Option<Arc<NoteService>>,
}

impl Module for NotesModule {
    fn name(&self) -> &'static str {
        "notes"
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

    fn limits(&self) -> Vec<autoxide_kernel::entitlement::Limit> {
        vec![service::NOTES_LIMIT]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let service = Arc::new(NoteService::new(context.outbox()?, context.entitlements()));
        context.usage_counter(&service::NOTES_LIMIT, Arc::new(service::NoteCounter))?;
        self.service = Some(service.clone());
        // `Arc<NoteService>` tự đổi thành `Arc<dyn Notes>`: module khác chỉ thấy trait.
        context.personal_data(service.clone())?;
        context.provide::<dyn api::Notes>(service)
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // Tài khoản bị xoá: xoá ghi chú của người đó.
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![Subscription::new(autoxide_kernel::privacy::USER_DELETED, service)]
        })
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}
