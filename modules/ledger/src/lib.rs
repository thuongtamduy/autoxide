//! Module `ledger` (bước 36 mục 2 và một nửa mục 7, D-342): sổ cái kép. Mỗi nút (đơn vị kế toán)
//! có một sổ; tài khoản theo mục đích (`revenue.sales`, `payable.seller_held`…); bút toán chỉ thêm,
//! sửa sai bằng bút toán đảo; số dư có sàn, chịu được ghi đồng thời; kỳ tháng khoá được; chuỗi băm
//! phát hiện sửa ngoài module. Sổ cái chỉ ghi nghĩa vụ — core không giữ tiền (D-187). Module khác
//! ghi sổ qua [`api::Ledger`]. Thêm (mục 4, 9): ba chế độ dòng tiền và tạm giữ ([`flows`]), chi trả
//! ([`payouts`]), VietQR và biến động số dư ngân hàng ([`bank`], [`vietqr`]).

pub mod api;
mod bank;
mod chain;
mod config;
mod entity;
mod events;
mod flows;
mod fx;
mod http;
mod http_flows;
mod http_fx;
mod http_payments;
mod journal;
mod migration;
mod payments;
mod payouts;
mod periods;
mod permissions;
mod posting;
mod provider;
mod reconcile;
mod service;
mod tools;
mod tools_flows;
mod vietqr;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_flows;
#[cfg(test)]
mod tests_fx;
#[cfg(test)]
mod tests_payments;
#[cfg(test)]
mod tests_vietqr;

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
            "ledger_flow_settings",
            "ledger_holds",
            "ledger_payouts",
            "ledger_bank_accounts",
            "ledger_bank_transactions",
            // Tỉ giá đã dùng cho chứng từ, bút toán ngoại tệ (Luật Kế toán 88/2015, Điều 11).
            "ledger_fx_rates",
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
        vec![
            autoxide_kernel::http::BodyLimit::new(
                axum::http::Method::POST,
                "/webhooks/{provider}",
                self.config.webhook_max_bytes,
            ),
            autoxide_kernel::http::BodyLimit::new(
                axum::http::Method::POST,
                "/bank-feeds/{provider}",
                self.config.webhook_max_bytes,
            ),
        ]
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
            payout_gateways: context.contributions::<dyn autoxide_kernel::payment::PayoutGateway>(),
            bank_feeds: context.contributions::<dyn autoxide_kernel::payment::BankFeed>(),
            rate_sources: context
                .contributions::<dyn autoxide_kernel::payment::ExchangeRateSource>(),
            tax: context.require::<dyn autoxide_tax::api::TaxEngine>().ok(),
            jobs: context.jobs(),
            config: self.config,
        });
        self.service = Some(service.clone());
        context.provide::<dyn api::Ledger>(Arc::new(provider::Provider(service)))
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http::routes(service.clone())
                .merge(http_payments::routes(service.clone()))
                .merge(http_flows::routes(service.clone()))
                .merge(http_fx::routes(service))
        })
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, |service| {
            let mut tools = tools::all(service);
            tools.extend(tools_flows::all(service));
            tools
        })
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
            jobs.push(JobRegistration::new(
                flows::RELEASE_HOLDS,
                Arc::new(flows::ReleaseHoldsJob(service.clone())),
            ));
            jobs.push(JobRegistration::new(
                payouts::CHECK_PAYOUTS,
                Arc::new(payouts::CheckPayoutsJob(service.clone())),
            ));
            jobs.push(JobRegistration::new(
                bank::MATCH_BANK_TRANSACTION,
                Arc::new(bank::MatchBankTransactionJob(service.clone())),
            ));
            jobs.push(JobRegistration::new(
                bank::POLL_BANK_FEED,
                Arc::new(bank::PollBankFeedJob(service.clone())),
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
            Schedule::new(flows::RELEASE_HOLDS, flows::RELEASE_HOLDS, Duration::from_secs(300)),
            Schedule::new(payouts::CHECK_PAYOUTS, payouts::CHECK_PAYOUTS, Duration::from_secs(300)),
            Schedule::new(
                bank::POLL_BANK_FEED,
                bank::POLL_BANK_FEED,
                Duration::from_secs(self.config.bank_feed_poll_seconds),
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
