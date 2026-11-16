//! Module `address` (bước 34 mục 5–6, D-193, D-314): danh mục đơn vị hành chính dùng chung mọi
//! khách (Việt Nam sau sáp nhập 2025: tỉnh → xã / phường / đặc khu, không cấp huyện), nhập từ
//! nguồn chính thức bằng lệnh `autoxide task address.import`, tra công khai qua REST / MCP, và
//! kiểm địa chỉ cho module khác qua [`api::AdministrativeUnits`].

pub mod api;
mod entity;
mod events;
mod http;
mod import;
mod migration;
mod service;
mod tools;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::service::AddressService;

pub fn module() -> impl Module {
    AddressModule::default()
}

#[derive(Default)]
struct AddressModule {
    service: Option<Arc<AddressService>>,
}

impl Module for AddressModule {
    fn name(&self) -> &'static str {
        "address"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        vec!["address_dataset", "address_unit"]
    }

    fn events(&self) -> Vec<EventInfo> {
        events::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let service =
            Arc::new(AddressService { databases: context.tenancy(), outbox: context.outbox()? });
        self.service = Some(service.clone());
        context.provide::<dyn api::AdministrativeUnits>(service)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        self.service.clone().map_or_else(Vec::new, |service| vec![import::task(service)])
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![autoxide_kernel::doctor::Check::new(
                "administrative_units",
                "An official list of administrative units (Vietnam) is imported, for checking addresses.",
                Arc::new(Dataset(service)),
            )]
        })
    }
}

/// `autoxide doctor`: bộ dữ liệu đang dùng (nguồn, ngày hiệu lực) hay chưa nhập.
struct Dataset(Arc<AddressService>);

#[async_trait::async_trait]
impl autoxide_kernel::doctor::Probe for Dataset {
    async fn probe(&self) -> autoxide_kernel::doctor::Finding {
        use autoxide_kernel::doctor::Finding;
        match self.0.latest("VN").await {
            Ok(Some(dataset)) => Finding::ok(format!(
                "{} ({} provinces, {} communes), in force from {}, from {} retrieved on {}.",
                dataset.title,
                dataset.province_count,
                dataset.commune_count,
                dataset.effective_from,
                dataset.source_url,
                dataset.retrieved_on
            )),
            Ok(None) => Finding::warning("No list of administrative units is imported: addresses cannot be checked.")
                .with_hint("Get the official list (see docs/ADDRESS.md) and run `autoxide task address.import path=<directory>`."),
            Err(error) => Finding::failed(format!("cannot read the administrative units: {error}")),
        }
    }
}
