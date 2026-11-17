//! Module `ledger` (bước 36 mục 2 và một nửa mục 7, D-342): sổ cái kép. Mỗi nút (đơn vị kế toán)
//! có một sổ; tài khoản theo mục đích (`revenue.sales`, `payable.seller_held`…); bút toán chỉ thêm,
//! sửa sai bằng bút toán đảo; số dư có sàn, chịu được ghi đồng thời; kỳ tháng khoá được; chuỗi băm
//! phát hiện sửa ngoài module. Sổ cái chỉ ghi nghĩa vụ — core không giữ tiền (D-187). Module khác
//! ghi sổ qua [`api::Ledger`].

pub mod api;
mod chain;
mod config;
mod entity;
mod events;
mod http;
mod http_payments;
mod migration;
mod payments;
mod periods;
mod permissions;
mod posting;
mod provider;
mod reconcile;
mod service;
mod tools;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_payments;

use std::sync::Arc;

use autoxide_kernel::access::Permission;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::service::LedgerService;

pub fn module() -> impl Module {
    LedgerModule::default()
}

#[derive(Default)]
struct LedgerModule {
    config: config::LedgerConfig,
    service: Option<Arc<LedgerService>>,
}

const SEAL_ENTRIES: &str = "ledger.seal_entries";

/// Niêm phong bút toán mới vào chuỗi băm của từng sổ (mỗi database, 10 giây).
struct SealEntries;

#[async_trait::async_trait]
impl JobHandler for SealEntries {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        LedgerService::seal_entries(&job.database).await.map(|_| ())
    }
}

impl Module for LedgerModule {
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("finance", "Finance", "wallet", 80)).item(
            NavItem::new("finance.ledger", "finance", "Ledger", 10)
                .permissions(&[crate::permissions::READ]),
        )
    }

    fn name(&self) -> &'static str {
        "ledger"
    }

    // Module nền: không gói nào tắt được (bước 35, D-323).
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    // Sổ kế toán phải lưu ít nhất 10 năm, kể cả khi xoá hẳn khách (Luật Kế toán 88/2015, Điều 41;
    // D-190, D-337).
    fn retained_tables(&self) -> Vec<&'static str> {
        vec![
            "ledger_books",
            "ledger_accounts",
            "ledger_account_balances",
            "ledger_entries",
            "ledger_lines",
            "ledger_periods",
            "ledger_period_balances",
            "ledger_chain_heads",
            "ledger_entry_seals",
            "ledger_payments",
            "ledger_refunds",
            "ledger_gateway_events",
            "ledger_statement_lines",
            "ledger_exceptions",
            "ledger_reconciliations",
        ]
    }

    fn configure(
        &mut self,
        reader: &mut autoxide_kernel::config::ConfigReader<'_>,
    ) -> Result<(), autoxide_kernel::config::Invalid> {
        self.config = config::read(reader)?;
        Ok(())
    }

    fn body_limits(&self) -> Vec<autoxide_kernel::http::BodyLimit> {
        vec![autoxide_kernel::http::BodyLimit::new(
            axum::http::Method::POST,
            "/webhooks/{provider}",
            self.config.webhook_max_bytes,
        )]
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
        let service = Arc::new(LedgerService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            directory: context.require::<dyn autoxide_organizations::api::OrgDirectory>().ok(),
            gateways: context.contributions::<dyn autoxide_kernel::payment::PaymentGateway>(),
            jobs: context.jobs(),
            config: self.config,
        });
        self.service = Some(service.clone());
        context.provide::<dyn api::Ledger>(Arc::new(provider::Provider(service)))
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http::routes(service.clone()).merge(http_payments::routes(service))
        })
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        let mut jobs = vec![JobRegistration::new(SEAL_ENTRIES, Arc::new(SealEntries))];
        if let Some(service) = &self.service {
            jobs.push(JobRegistration::new(
                payments::PROCESS_GATEWAY_EVENT,
                Arc::new(payments::ProcessGatewayEventJob(service.clone())),
            ));
            jobs.push(JobRegistration::new(
                payments::EXPIRE_PAYMENTS,
                Arc::new(payments::ExpirePaymentsJob(service.clone())),
            ));
            jobs.push(JobRegistration::new(
                reconcile::RECONCILE_JOB,
                Arc::new(reconcile::ReconcileJob(service.clone())),
            ));
        }
        jobs
    }

    fn schedules(&self) -> Vec<Schedule> {
        use std::time::Duration;
        vec![
            Schedule::new(SEAL_ENTRIES, SEAL_ENTRIES, Duration::from_secs(10)),
            Schedule::new(
                payments::EXPIRE_PAYMENTS,
                payments::EXPIRE_PAYMENTS,
                Duration::from_secs(60),
            ),
            Schedule::new(
                reconcile::RECONCILE_JOB,
                reconcile::RECONCILE_JOB,
                Duration::from_secs(86_400),
            ),
        ]
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.service.as_ref().map_or_else(Vec::new, |service| {
            vec![
                autoxide_kernel::doctor::Check::new(
                    "chain",
                    "No ledger entry was changed or removed after it was sealed, and new entries \
                     get sealed (hash chain).",
                    Arc::new(chain::ChainProbe { databases: service.databases.clone() }),
                )
                .with_timeout(chain::VERIFY_TIMEOUT),
                autoxide_kernel::doctor::Check::new(
                    "balances",
                    "Every ledger account balance matches the sum of its entry lines.",
                    Arc::new(chain::BalancesProbe { databases: service.databases.clone() }),
                )
                .with_timeout(chain::VERIFY_TIMEOUT),
            ]
        })
    }
}
