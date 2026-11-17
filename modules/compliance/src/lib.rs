//! Module `compliance` (bước 37, D-347): tuân thủ pháp luật Việt Nam ở mức nền.
//!
//! - Định danh (KYC) qua nhà cung cấp ([`autoxide_kernel::identity::IdentityVerifier`]) hay duyệt tay;
//!   module khác chặn hoạt động cần định danh qua [`api::KycGate`].
//! - Quy tắc pháp lý là dữ liệu có ngày hiệu lực (gói `rules/vn.json`, nhập vào mặt điều khiển).
//! - Văn bản pháp lý có phiên bản, bằng chứng chấp nhận ([`api::AcceptanceGate`]).
//! - Sổ đồng ý theo mục đích ([`api::ConsentLedger`]).
//! - Xác thực tài khoản đăng nội dung (NĐ 147/2024, D-356): số điện thoại bằng mã SMS một lần, mức
//!   định danh từ KYC ([`api::AccountVerification`], [`api::PostingGate`], `PostingVerifier` của kernel).
//!
//! Mọi căn cứ pháp lý ghi trong quy tắc và D-347 đều cần đối chiếu văn bản gốc / người có chuyên môn
//! pháp lý trước khi chạy thật.

pub mod api;
mod config;
mod consents;
mod documents;
mod entity;
mod events;
mod http_kyc;
mod http_legal;
mod http_verification;
mod kyc;
mod migration;
mod permissions;
mod provider;
mod service;
mod tools;
mod verification;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_verification;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::job::{JobContext, JobHandler, JobRegistration, Schedule};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, StartContext};
use utoipa_axum::router::OpenApiRouter;

use crate::service::ComplianceService;

/// Job nhập gói quy tắc đi kèm vào mặt điều khiển.
const RULES_REFRESH: &str = "compliance.rules_refresh";

pub fn module() -> impl Module {
    ComplianceModule { service: None, config: config::Config::default() }
}

struct ComplianceModule {
    service: Option<Arc<ComplianceService>>,
    config: config::Config,
}

struct RulesRefreshJob(Arc<ComplianceService>);

#[async_trait::async_trait]
impl JobHandler for RulesRefreshJob {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.0.import_bundled().await.map(|_| ())
    }
}

impl Module for ComplianceModule {
    fn navigation(&self) -> autoxide_kernel::module::navigation::Navigation {
        use autoxide_kernel::module::navigation::{NavGroup, NavItem, Navigation};
        Navigation::default().group(NavGroup::new("compliance", "Compliance", "shield", 100)).item(
            NavItem::new("compliance.overview", "compliance", "Compliance", 10).permissions(&[
                crate::permissions::KYC_READ,
                crate::permissions::DOCUMENTS_READ,
                crate::permissions::PRIVACY_READ,
                crate::permissions::RULES_READ,
            ]),
        )
    }

    fn name(&self) -> &'static str {
        "compliance"
    }

    // Tuân thủ pháp luật không phải tính năng bán theo gói (Luật số hai của CLAUDE.md).
    fn always_enabled(&self) -> bool {
        true
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn control_plane_tables(&self) -> Vec<&'static str> {
        vec!["compliance_rules"]
    }

    // Bằng chứng chấp nhận và đồng ý phải giữ để chứng minh, kể cả khi xoá hẳn khách.
    fn retained_tables(&self) -> Vec<&'static str> {
        vec![
            "compliance_documents",
            "compliance_document_versions",
            "compliance_acceptances",
            "compliance_consents",
            "compliance_consent_records",
        ]
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = config::read(reader)?;
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
        let service = Arc::new(ComplianceService {
            databases: context.tenancy(),
            outbox: context.outbox()?,
            jobs: context.jobs(),
            verifier: context.require::<dyn autoxide_kernel::identity::IdentityVerifier>().ok(),
            callback_retention_days: self.config.callback_retention_days,
            sms: context.require::<dyn autoxide_kernel::sms::SmsSender>().ok(),
            secret: (*context.require::<autoxide_kernel::secret::SecretKey>()?).clone(),
            otp: self.config.otp,
        });
        self.service = Some(service.clone());
        let provider = Arc::new(provider::Provider(service));
        context.provide::<dyn api::KycGate>(provider.clone())?;
        context.provide::<dyn api::AcceptanceGate>(provider.clone())?;
        context.provide::<dyn api::ConsentLedger>(provider.clone())?;
        context.provide::<dyn api::PostingGate>(provider.clone())?;
        context.provide::<dyn api::AccountVerification>(provider.clone())?;
        // Bộ hỏi "đã xác thực để đăng nội dung chưa" của kernel (D-202) — `comments` hỏi qua nó.
        context.posting_verifier(provider.clone())?;
        context.provide::<dyn api::ComplianceRules>(provider)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http_kyc::routes(service.clone())
                .merge(http_legal::routes(service.clone()))
                .merge(http_verification::routes(service))
        })
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, tools::all)
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(kyc::KYC_PROCESS, Arc::new(kyc::ProcessJob(service.clone()))),
                JobRegistration::new(kyc::KYC_EXPIRY, Arc::new(kyc::ExpiryJob(service.clone()))),
                JobRegistration::new(RULES_REFRESH, Arc::new(RulesRefreshJob(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![
            Schedule::new(kyc::KYC_EXPIRY, kyc::KYC_EXPIRY, Duration::from_secs(86_400)),
            Schedule::new(RULES_REFRESH, RULES_REFRESH, Duration::from_secs(3_600))
                .on_control_only(),
        ]
    }
}
