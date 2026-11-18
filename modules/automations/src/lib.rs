//! Module `automations`: luật "khi … thì …" do chủ cửa hàng (và AI agent) tự đặt mà không viết code
//! (bước 58, D-220, D-229).
//!
//! - Luật = kích hoạt (event / lịch / gọi tay / số liệu bất thường) + điều kiện (cây toán tử cố
//!   định) + hành động (tool MCP đã có, cộng `wait`, `stop_if`, `notify`).
//! - Hành động gọi tool qua sổ tool của kernel (`AutomationDirectory`) với quyền HIỆN TẠI của người
//!   tạo luật — luật không làm được điều REST / MCP với cùng permission không làm được (luật kiến
//!   trúc 8).
//! - Đăng ký CUỐI CÙNG trong `crates/app/src/modules.rs`: nghe event và gọi tool của mọi module.

mod config;
mod definition;
mod entity;
mod events;
mod http;
mod metrics;
mod migration;
mod notices;
mod permissions;
mod runner;
mod service;
mod tools;
mod work;

mod relocation;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
use autoxide_kernel::job::{JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::config::AutomationsConfig;
use crate::service::Automations;

/// Module `automations`, để binary đăng ký trong `crates/app/src/modules.rs`.
pub fn module() -> impl Module {
    AutomationsModule::default()
}

#[derive(Default)]
struct AutomationsModule {
    config: AutomationsConfig,
    /// Có sau `start`.
    service: Option<Arc<Automations>>,
    /// Nguồn số liệu báo cáo của mọi module (cộng cho kích hoạt `metric`).
    sources: Vec<autoxide_kernel::report::FactSource>,
    /// Bản service cho test của crate gọi thẳng (lịch, kiểm số liệu ở một mốc chọn trước).
    shared: Arc<std::sync::OnceLock<Arc<Automations>>>,
}

/// Module và tay cầm tới service của nó (có sau `start`), cho test của crate.
#[cfg(test)]
pub(crate) fn module_with_service() -> (impl Module, Arc<std::sync::OnceLock<Arc<Automations>>>) {
    let module = AutomationsModule::default();
    let shared = module.shared.clone();
    (module, shared)
}

impl Module for AutomationsModule {
    fn name(&self) -> &'static str {
        "automations"
    }

    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("settings", "Settings", "gear", 110)).item(
            NavItem::new(work::SCREEN, "settings", "Automations", 60)
                .permissions(&[permissions::AUTOMATIONS_READ, permissions::AUTOMATIONS_MANAGE])
                .counter("automations.approvals"),
        )
    }

    fn description(&self) -> &'static str {
        "Automation rules: when an event, a schedule, a manual call or an unusual figure happens \
         and the conditions hold, run MCP tools with the permissions of the rule's author."
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

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = config::read(reader)?;
        Ok(())
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let sources: Vec<_> =
            context.fact_sources().into_iter().map(|(_, source)| source).collect();
        self.sources = sources.clone();
        let service = Automations {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            jobs: context.jobs(),
            directory: context.automation_directory(),
            events: context.events().clone(),
            templates: (*context.require::<autoxide_kernel::template::TemplateEngine>()?).clone(),
            authenticator: context.require::<dyn autoxide_kernel::access::Authenticator>().ok(),
            orgs: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            idempotency: context
                .require::<dyn autoxide_kernel::idempotency::IdempotencyStore>()
                .ok(),
            notifier: context.require::<dyn autoxide_notifications::api::Notifier>().ok(),
            metrics: metrics::MetricSources::new(&sources, context.summary_metrics()),
            config: self.config,
        };
        for source in work::sources() {
            context.work_source(source)?;
        }
        if let Some(notifier) = &service.notifier {
            notices::declare(notifier.as_ref())?;
        }
        let service = Arc::new(service);
        let _ = self.shared.set(service.clone());
        self.service = Some(service);
        Ok(())
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        // Mọi event của mọi module: luật nghe event nào là dữ liệu của khách, không biết trước.
        let Some(service) = &self.service else { return Vec::new() };
        let mut subscriptions = vec![Subscription::new(
            Subscription::ALL,
            Arc::new(runner::triggers::EventTrigger(service.clone())),
        )];
        for source in &self.sources {
            let handler = metrics::record::FactHandler { source: source.clone() };
            subscriptions.push(Subscription::new(source.event(), Arc::new(handler)));
        }
        subscriptions
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(
                    runner::RUN_JOB,
                    Arc::new(runner::RunHandler(service.clone())),
                ),
                JobRegistration::new(
                    runner::schedule::TICK,
                    Arc::new(runner::schedule::TickJob(service.clone())),
                ),
                JobRegistration::new(
                    runner::schedule::PRUNE,
                    Arc::new(runner::schedule::PruneJob(service)),
                ),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![
            Schedule::new(
                runner::schedule::TICK,
                runner::schedule::TICK,
                Duration::from_secs(self.config.tick_seconds),
            ),
            Schedule::new(
                runner::schedule::PRUNE,
                runner::schedule::PRUNE,
                Duration::from_secs(86_400),
            ),
        ]
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        relocation::relocation()
    }
}
