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

use autoxide_kernel::db::RecordInfo;
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Manifest, Module, StartContext};

use crate::service::AddressService;

pub fn module() -> impl Module {
    AddressModule
}

struct AddressModule;

// Hợp đồng module (D-20261126-module-manifest): `manifest` khai mọi thứ tĩnh, `start` dựng service rồi đăng ký
// những gì cần service (route, tool MCP, lệnh `autoxide task`, kiểm `autoxide doctor`, trait cho module khác).
impl Module for AddressModule {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            "address",
            "Administrative units of Vietnam (provinces and communes after the 2025 merger) and \
             address checks, shared by every tenant.",
        )
        // Module nền: không gói nào tắt được (bước 35, D-323).
        .always_enabled()
        .migrations(migration::all)
        .control_plane_tables(["address_dataset", "address_unit"])
        .records([RecordInfo::of::<entity::Dataset>(), RecordInfo::of::<entity::Unit>()])
        .events(events::all())
        .translation("vi", include_str!("../locales/vi.json"))
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let service = Arc::new(AddressService {
            control: context.kernel()?,
            databases: context.kernel()?,
            outbox: context.kernel()?,
        });
        context.routes(http::routes(service.clone()))?;
        context.tools(tools::all(&service))?;
        context.task(import::task(service.clone()))?;
        context.check(autoxide_kernel::doctor::Check::new(
            "administrative_units",
            "An official list of administrative units (Vietnam) is imported, for checking addresses.",
            Arc::new(Dataset(service.clone())),
        ))?;
        context.provide::<dyn api::AdministrativeUnits>(service)
    }
}

/// `autoxide doctor`: bộ dữ liệu đang dùng (nguồn, ngày hiệu lực) hay chưa nhập.
struct Dataset(Arc<AddressService>);

#[autoxide_kernel::async_trait]
impl autoxide_kernel::doctor::Probe for Dataset {
    async fn probe(&self) -> autoxide_kernel::doctor::Finding {
        use autoxide_kernel::doctor::Finding;
        let bundled = import::bundled();
        match self.0.latest("VN").await {
            Ok(Some(dataset))
                if bundled.as_ref().is_ok_and(|(manifest, sha)| {
                    *sha != dataset.sha256 && manifest.effective_from > dataset.effective_from
                }) =>
            {
                Finding::warning(format!(
                    "The list in use ({}, in force from {}) is older than the one shipped with this \
                     server.",
                    dataset.title, dataset.effective_from
                ))
                .with_hint("Run `autoxide task address.import` to import the shipped list.")
            }
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
                .with_hint("Run `autoxide task address.import` to import the list shipped with the server (see docs/ADDRESS.md)."),
            Err(error) => Finding::failed(format!("cannot read the administrative units: {error}")),
        }
    }
}
