//! Module `tax` (bước 36 mục 5, D-188, D-341): engine thuế. Nhóm thuế và thuế suất có ngày hiệu lực
//! là **dữ liệu** (gói dữ liệu, gói "vn" đi kèm), không viết cứng trong code; hồ sơ thuế theo nút
//! (kế thừa theo cây tổ chức); thuế khấu trừ thay của sàn; yêu cầu hoá đơn (máy tính tiền). Module
//! khác tính thuế qua [`api::TaxEngine`].

pub mod api;
mod config;
mod engine;
mod entity;
mod events;
mod http;
mod migration;
mod pack;
mod permissions;
mod provider;
mod service;
mod tools;

#[cfg(test)]
mod tests;

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::service::{TablesCache, TaxService};

pub fn module() -> impl Module {
    TaxModule::default()
}

#[derive(Default)]
struct TaxModule {
    warning_days: Option<i64>,
    service: Option<Arc<TaxService>>,
}

const IMPORT_BUNDLED: &str = "tax.import_bundled";

/// Nhập gói đi kèm khi nó mới hơn bản trong database (hằng giờ, và ngay sau khi máy chủ chạy).
struct ImportBundled(Arc<TaxService>);

#[async_trait::async_trait]
impl JobHandler for ImportBundled {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.import_bundled().await.map(|_| ())
    }
}

impl Module for TaxModule {
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("finance", "Finance", "wallet", 80)).item(
            NavItem::new("finance.tax", "finance", "Tax", 30)
                .permissions(&[crate::permissions::READ]),
        )
    }

    fn name(&self) -> &'static str {
        "tax"
    }

    // Tách / gộp khách (bước 38, D-20261118-step38-relocation).
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        use autoxide_kernel::tenancy::Relocation;
        Relocation::new().by_org(&["tax_profiles"])
    }

    fn description(&self) -> &'static str {
        "Tax engine: tax groups and rates as dated data, tax profiles per organization unit, \
         withholding and invoice requirements."
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        vec![
            "tax_packs",
            "tax_jurisdictions",
            "tax_categories",
            "tax_rates",
            "tax_withholding_rates",
            "tax_rules",
        ]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.warning_days = Some(config::read(reader)?);
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
        let service = Arc::new(TaxService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            tables: TablesCache::new(),
            units: context.require::<dyn autoxide_address::api::AdministrativeUnits>().ok(),
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            warning_days: self.warning_days.unwrap_or(60),
        });
        self.service = Some(service.clone());
        context.provide::<dyn api::TaxEngine>(Arc::new(provider::Provider(service)))
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![JobRegistration::new(IMPORT_BUNDLED, Arc::new(ImportBundled(service)))]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![
            Schedule::new(IMPORT_BUNDLED, IMPORT_BUNDLED, std::time::Duration::from_secs(3600))
                .on_control_only(),
        ]
    }

    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        self.service.clone().map_or_else(Vec::new, |service| vec![pack::task(service)])
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                autoxide_kernel::doctor::Check::new(
                    "tax_rates",
                    "Tax rates are imported, and none ends soon without a rate after it.",
                    Arc::new(ExpiringRates(service.clone())),
                ),
                autoxide_kernel::doctor::Check::new(
                    "tax_time_zones",
                    "Every imported tax jurisdiction declares the time zone of its tax dates.",
                    Arc::new(MissingTimeZones(service)),
                ),
            ]
        })
    }
}

/// `autoxide doctor`: thuế suất sắp hết hạn mà chưa có bản kế tiếp cho cùng nhóm.
struct ExpiringRates(Arc<TaxService>);

#[async_trait::async_trait]
impl autoxide_kernel::doctor::Probe for ExpiringRates {
    async fn probe(&self) -> autoxide_kernel::doctor::Finding {
        use autoxide_kernel::doctor::Finding;
        match self.0.expiring_rates(Timestamp::now().date_utc()).await {
            Ok(None) => Finding::warning("No tax data pack is imported: taxes cannot be computed.")
                .with_hint("Run `autoxide task tax.import_pack` to import the pack shipped with the server."),
            Ok(Some(expiring)) if expiring.is_empty() => Finding::ok("Every tax rate in force has a rate after it, or does not end."),
            Ok(Some(expiring)) => Finding::warning(format!(
                "Tax rates end soon with no rate after them: {}.",
                expiring.join("; ")
            ))
            .with_hint("Import a newer tax data pack (`autoxide task tax.import_pack path=…`)."),
            Err(error) => Finding::failed(format!("cannot read the tax rates: {error}")),
        }
    }
}

/// `autoxide doctor`: vùng thuế đã nhập mà không khai múi giờ đang hiệu lực (quy tắc `tax_date.time_zone`):
/// ngày thuế của người bán ở đó lặng lẽ theo múi giờ của nút, mặc định UTC (LG-2).
struct MissingTimeZones(Arc<TaxService>);

#[async_trait::async_trait]
impl autoxide_kernel::doctor::Probe for MissingTimeZones {
    async fn probe(&self) -> autoxide_kernel::doctor::Finding {
        use autoxide_kernel::doctor::Finding;
        match self.0.jurisdictions_without_time_zone(Timestamp::now()).await {
            Ok(missing) if missing.is_empty() => {
                Finding::ok("Every imported tax jurisdiction declares its time zone.")
            }
            Ok(missing) => Finding::warning(format!(
                "Tax jurisdictions without a time zone rule in force: {}. Tax points and invoice dates of \
                 sellers there follow the unit time zone (`organizations.timezone`, UTC by default).",
                missing.join(", ")
            ))
            .with_hint("Add a `tax_date.time_zone` rule to the tax data pack and import it."),
            Err(error) => Finding::failed(format!("cannot read the tax rules: {error}")),
        }
    }
}
