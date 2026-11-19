//! Module `reporting`: báo cáo gộp của cây khách hàng (bước 38, D-185,
//! D-20261118-step38-report-facts).
//!
//! Hai chặng, không có transaction xuyên database:
//!
//! 1. Rút (`extract.rs`): mỗi module khai số liệu cho event của mình (`Module::facts`); module này
//!    nghe đúng các event đó, ghi `reporting_local_facts` ở database của khách nguồn.
//! 2. Đẩy (`forward.rs`): lịch `reporting.forward` (mỗi `AUTOXIDE_REPORTING_FORWARD_SECONDS`, trên
//!    từng database) chép số liệu lên kho của cây ở database của khách GỐC — kể cả khi khách con ở
//!    database khác — và cộng dồn theo ngày.
//!
//! Xem (`service.rs`): khách X với `reporting.read` độ sâu `tenant` thấy X và mọi khách cấp dưới;
//! độ sâu hẹp hơn chỉ thấy số liệu của X ở các nút của mình. Đăng ký CUỐI CÙNG trong
//! `crates/app/src/modules.rs` (cần `dyn Ledger` cho tỉ giá, `dyn OrgDirectory` cho múi giờ).

mod config;
mod entity;
mod extract;
mod forward;
mod http;
mod migration;
mod permissions;
mod rehome;
mod service;
mod tools;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_groups;
#[cfg(test)]
mod tests_scope;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Subscription;
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use autoxide_kernel::report::FactSource;
use utoipa_axum::router::OpenApiRouter;

use crate::config::ReportingConfig;
use crate::service::Reporting;

/// Module `reporting`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    ReportingModule::default()
}

#[derive(Default)]
struct ReportingModule {
    config: ReportingConfig,
    /// Có sau `start`.
    service: Option<Arc<Reporting>>,
    sources: Vec<FactSource>,
    /// Hàng đợi job của module (xếp `reporting.rehome`); có sau `start`.
    jobs: Option<autoxide_kernel::job::JobQueue>,
}

impl Module for ReportingModule {
    fn name(&self) -> &'static str {
        "reporting"
    }

    // Tách / gộp khách (bước 38, D-20261118-step38-relocation).
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        use autoxide_kernel::tenancy::Relocation;
        Relocation::new()
            .by_org(&["reporting_local_facts"])
            // Kho của cây ở database của khách gốc: dời bằng job `reporting.rehome` khi nghe
            // `kernel.relocation_finished`, không theo dòng của khách bị tách / gộp.
            .ephemeral(&[
                "reporting_facts",
                "reporting_fact_dimensions",
                "reporting_daily",
                "reporting_rehomes",
            ])
    }

    fn description(&self) -> &'static str {
        "Reports across a customer tree: figures from every tenant (also on other databases), \
         summed per day, tenant, unit and currency."
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn translations(&self) -> Vec<(&'static str, &'static str)> {
        vec![("vi", include_str!("../locales/vi.json"))]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = config::read(reader)?;
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let sources = context.fact_sources();
        let mut service = Reporting::new(
            context.tenancy(),
            context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            context.require::<dyn autoxide_ledger::api::Ledger>().ok(),
            &sources,
            self.config,
        );
        // Nhóm nút ngoài cây (D-232) để lọc / gom báo cáo.
        service.groups = context.require::<dyn autoxide_organizations::api::OrgGroups>().ok();
        let service = Arc::new(service);
        self.sources = sources.into_iter().map(|(_, source)| source).collect();
        self.service = Some(service);
        self.jobs = Some(context.jobs());
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        let Some(service) = &self.service else { return Vec::new() };
        let mut subscriptions: Vec<Subscription> = self
            .sources
            .iter()
            .map(|source| {
                let handler =
                    extract::FactHandler { source: source.clone(), service: service.clone() };
                Subscription::new(source.event(), Arc::new(handler))
            })
            .collect();
        if let Some(jobs) = &self.jobs {
            let listener = rehome::RelocationListener { jobs: jobs.clone() };
            subscriptions.push(Subscription::new(rehome::RELOCATION_FINISHED, Arc::new(listener)));
        }
        subscriptions
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        let Some(service) = &self.service else { return Vec::new() };
        vec![
            JobRegistration::new(forward::FORWARD, Arc::new(forward::ForwardJob(service.clone()))),
            JobRegistration::new(
                forward::PRUNE_LOCAL,
                Arc::new(forward::PruneLocalJob(service.clone())),
            ),
            JobRegistration::new(rehome::REHOME, Arc::new(rehome::RehomeJob(service.clone()))),
        ]
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![
            Schedule::new(
                forward::FORWARD,
                forward::FORWARD,
                Duration::from_secs(self.config.forward_seconds),
            ),
            Schedule::new(forward::PRUNE_LOCAL, forward::PRUNE_LOCAL, Duration::from_secs(86_400)),
        ]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }
}
