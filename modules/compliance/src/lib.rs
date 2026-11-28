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
mod blocks;
mod cases;
mod checklist;
mod complaint_intake;
mod config;
mod consents;
mod consumer;
mod disclosures;
mod documents;
mod entity;
mod events;
mod http_cases;
mod http_kyc;
mod http_legal;
mod http_moit;
mod http_privacy;
mod http_tracking;
mod http_verification;
mod incidents;
mod kyc;
mod legal;
mod migration;
mod overview;
mod permissions;
mod phone_lock;
mod phone_locks;
mod privacy;
mod provider;
mod service;
mod sign_in_codes;
mod sms_templates;
mod subject_requests;
mod tools;
mod tools_cases;
mod tools_privacy;
mod tracking;
mod verification;
mod websites;

#[cfg(test)]
mod sign_in_codes_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_cases;
#[cfg(test)]
mod tests_moit;
#[cfg(test)]
mod tests_pages;
#[cfg(test)]
mod tests_privacy;
#[cfg(test)]
mod tests_sms_templates;
#[cfg(test)]
mod tests_tracking;
#[cfg(test)]
mod tests_verification;

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Permission;
use autoxide_kernel::config::{ConfigReader, Invalid};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{EventInfo, Subscription};
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
                crate::permissions::LEGAL_READ,
                crate::permissions::CASES_READ,
            ]),
        )
    }

    fn name(&self) -> &'static str {
        "compliance"
    }

    // Tách / gộp khách (bước 38, D-20261118-step38-relocation).
    fn relocation(&self) -> autoxide_kernel::tenancy::Relocation {
        use autoxide_kernel::tenancy::Relocation;
        Relocation::new()
            .copy_on_split(&["compliance_policies", "compliance_documents", "compliance_purposes"])
            .follow("compliance_document_versions", "document_id", "compliance_documents")
            .by_org(&[
                "compliance_legal_profiles",
                "compliance_websites",
                "compliance_tracking_tags",
                "compliance_cookie_settings",
                "compliance_cookie_consents",
            ])
            .table(
                "compliance_website_pages",
                autoxide_kernel::tenancy::TablePolicy::FollowParent {
                    parent_table: "compliance_websites",
                    column: "website_id",
                    parent_column: "id",
                },
            )
            .table(
                "compliance_cases",
                autoxide_kernel::tenancy::TablePolicy::ByOrg { column: "respondent_org_id" },
            )
            .follow("compliance_case_respondents", "case_id", "compliance_cases")
            .follow("compliance_case_entries", "case_id", "compliance_cases")
            .follow("compliance_disclosures", "case_id", "compliance_cases")
            // Dữ liệu cá nhân theo khách (định danh, đồng ý, yêu cầu của chủ thể): ở lại khách nguồn
            // khi tách (D-20261118-step38-relocation), chuyển khi gộp.
            .stay(&[
                "compliance_kyc_profiles",
                "compliance_acceptances",
                "compliance_consents",
                "compliance_account_verifications",
                "compliance_incidents",
                "compliance_subject_requests",
            ])
            .follow("compliance_kyc_attempts", "profile_id", "compliance_kyc_profiles")
            .follow("compliance_consent_records", "consent_id", "compliance_consents")
            .follow("compliance_incident_entries", "incident_id", "compliance_incidents")
            .ephemeral(&[
                "compliance_kyc_callbacks",
                "compliance_phone_challenges",
                "compliance_phone_known_clients",
                "compliance_phone_failures",
                "compliance_throttle",
                "compliance_antispam_nonces",
            ])
    }

    fn description(&self) -> &'static str {
        "Vietnamese compliance basics: identity checks, legal documents and acceptances, \
         website registration, personal data requests and incidents."
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

    // Bằng chứng chấp nhận và đồng ý phải giữ để chứng minh, kể cả khi xoá hẳn khách; sổ cung cấp
    // thông tin cho cơ quan cũng vậy.
    fn retained_tables(&self) -> Vec<&'static str> {
        vec![
            "compliance_disclosures",
            "compliance_documents",
            "compliance_document_versions",
            "compliance_acceptances",
            "compliance_consents",
            "compliance_consent_records",
        ]
    }

    fn tenant_settings(&self) -> Vec<autoxide_kernel::tenancy::TenantSetting> {
        let mut settings = sms_templates::all();
        settings.push(config::CASE_TRACKING_LINK);
        settings.push(config::KYC_RETURN_ORIGINS);
        settings
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = config::read(reader)?;
        Ok(())
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn blocks(&self) -> Vec<autoxide_kernel::blocks::BlockType> {
        blocks::all()
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
            settings: context.require::<dyn autoxide_kernel::tenancy::TenantSettings>()?,
            domains: context.require::<autoxide_kernel::tenancy::TenantDomains>().ok(),
            secret: (*context.require::<autoxide_kernel::secret::SecretKey>()?).clone(),
            otp: self.config.otp,
            units: context.require::<dyn autoxide_address::api::AdministrativeUnits>().ok(),
            resources: context.resources(),
            templates: (*context.require::<autoxide_kernel::template::TemplateEngine>()?).clone(),
            facts: Arc::new(checklist::FactRegistry::default()),
            mailer: context.require::<dyn autoxide_kernel::mail::Mailer>().ok(),
            translations: context.translations().clone(),
            personal_data: context.personal_data_export(),
            retention: context.retention_catalog(),
            cookie_consents_per_hour: self.config.cookie_consents_per_hour,
            guard: (*context.require::<autoxide_kernel::antispam::SpamGuard>()?).clone(),
            public_reports_per_hour: self.config.public_reports_per_hour,
            case_tracking_url: self.config.case_tracking_url.clone(),
            purposes_tenant_max: self.config.purposes_tenant_max,
            links: context
                .require::<autoxide_kernel::tenancy::SiteLinks>()
                .map(|links| (*links).clone())
                .unwrap_or_default(),
        });
        // Chân trang pháp lý mà `content` tự gắn dưới mọi trang dựng từ khối (bước 67).
        context.block_data(
            blocks::LEGAL_FOOTER.name,
            Arc::new(blocks::LegalFooter(service.clone())),
        )?;
        self.service = Some(service.clone());
        context.personal_data(Arc::new(privacy::CompliancePersonalData))?;
        // Mã đăng nhập bằng số cho `auth` (đăng ký trước): không có `auth` (chương trình rút gọn) thì bỏ qua.
        if let Ok(extensions) = context.require::<dyn autoxide_auth::api::AuthExtensions>() {
            extensions.set_phone_codes(Arc::new(sign_in_codes::SignInCodes(service.clone())))?;
        }
        let provider = Arc::new(provider::Provider(service));
        context.provide::<dyn api::KycGate>(provider.clone())?;
        context.provide::<dyn api::AcceptanceGate>(provider.clone())?;
        context.provide::<dyn api::ConsentLedger>(provider.clone())?;
        context.provide::<dyn api::PostingGate>(provider.clone())?;
        context.provide::<dyn api::AccountVerification>(provider.clone())?;
        context.provide::<dyn api::LegalProfiles>(provider.clone())?;
        context.provide::<dyn api::ComplianceFacts>(provider.clone())?;
        context.provide::<dyn api::ModerationPolicy>(provider.clone())?;
        // Phiếu hỗ trợ chuyển thành khiếu nại người tiêu dùng (bước 53).
        context.provide::<dyn api::ComplaintIntake>(provider.clone())?;
        // Bộ hỏi "đã xác thực để đăng nội dung chưa" của kernel (D-202) — `comments` hỏi qua nó.
        context.posting_verifier(provider.clone())?;
        context.provide::<dyn api::ComplianceRules>(provider)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, |service| {
            http_cases::routes(service.clone())
                .merge(http_kyc::routes(service.clone()))
                .merge(http_legal::routes(service.clone()))
                .merge(http_moit::routes(service.clone()))
                .merge(http_privacy::routes(service.clone()))
                .merge(http_tracking::routes(service.clone()))
                .merge(http_verification::routes(service.clone()))
                .merge(phone_locks::routes(service.clone()))
                .merge(overview::routes(service))
        })
    }

    fn tools(&self) -> Vec<Tool> {
        self.service.as_ref().map_or_else(Vec::new, |service| {
            let mut all = tools::all(service);
            all.push(phone_locks::tool(service));
            all
        })
    }

    fn tasks(&self) -> Vec<autoxide_kernel::task::Task> {
        self.service.clone().map_or_else(Vec::new, |service| vec![phone_locks::task(service)])
    }

    fn checks(&self) -> Vec<autoxide_kernel::doctor::Check> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                autoxide_kernel::doctor::Check::new(
                    "moit_readiness",
                    "Every website has a legal profile and, before it is submitted to the Ministry of \
                     Industry and Trade (Vietnam), nothing its checklist needs is missing.",
                    Arc::new(checklist::ReadinessProbe {
                        databases: service.databases.clone(),
                        service: service.clone(),
                    }),
                )
                .with_timeout(Duration::from_secs(60)),
                autoxide_kernel::doctor::Check::new(
                    "retention",
                    "No kind of personal data is kept forever or longer than the legal rule \
                     `privacy.retention_limits` allows.",
                    Arc::new(privacy::RetentionProbe(service.clone())),
                ),
                autoxide_kernel::doctor::Check::new(
                    "data_residency",
                    "Every tenant that must keep its data in one country is on a placement in that \
                     country.",
                    Arc::new(privacy::ResidencyProbe(service.clone())),
                ),
                autoxide_kernel::doctor::Check::new(
                    "privacy_deadlines",
                    "No personal data breach is past its 72-hour reporting deadline and no data \
                     subject request is past its due date.",
                    Arc::new(privacy::DeadlinesProbe(service)),
                )
                .with_timeout(Duration::from_secs(60)),
            ]
        })
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        self.service.clone().map_or_else(Vec::new, |service| {
            vec![
                JobRegistration::new(
                    cases::CASE_DEADLINES,
                    Arc::new(cases::DeadlinesJob(service.clone())),
                ),
                JobRegistration::new(kyc::KYC_PROCESS, Arc::new(kyc::ProcessJob(service.clone()))),
                JobRegistration::new(kyc::KYC_EXPIRY, Arc::new(kyc::ExpiryJob(service.clone()))),
                JobRegistration::new(
                    incidents::INCIDENT_DEADLINES,
                    Arc::new(incidents::DeadlinesJob(service.clone())),
                ),
                JobRegistration::new(
                    tracking::CONSENT_PROOF_PURGE,
                    Arc::new(tracking::PurgeJob(service.clone())),
                ),
                JobRegistration::new(RULES_REFRESH, Arc::new(RulesRefreshJob(service))),
            ]
        })
    }

    fn schedules(&self) -> Vec<Schedule> {
        vec![
            Schedule::new(kyc::KYC_EXPIRY, kyc::KYC_EXPIRY, Duration::from_secs(86_400)),
            Schedule::new(
                incidents::INCIDENT_DEADLINES,
                incidents::INCIDENT_DEADLINES,
                Duration::from_secs(900),
            ),
            Schedule::new(
                tracking::CONSENT_PROOF_PURGE,
                tracking::CONSENT_PROOF_PURGE,
                Duration::from_secs(86_400),
            ),
            Schedule::new(cases::CASE_DEADLINES, cases::CASE_DEADLINES, Duration::from_secs(3_600)),
            Schedule::new(RULES_REFRESH, RULES_REFRESH, Duration::from_secs(3_600))
                .on_control_only(),
        ]
    }

    fn subscriptions(&self) -> Vec<Subscription> {
        if self.service.is_none() {
            return Vec::new();
        }
        vec![Subscription::new(
            autoxide_kernel::privacy::USER_DELETED,
            Arc::new(privacy::UserDeletedListener),
        )]
    }

    // Chế độ "nghỉ" (bước 70 P9): module tắt vẫn xuất và xoá dữ liệu của người dùng (số đã xác thực, mã
    // một lần, liên kết đồng ý cookie, vụ việc họ gửi).
    fn dormant(
        &self,
        context: &mut autoxide_kernel::module::DormantContext<'_>,
    ) -> Result<(), Error> {
        context.personal_data(Arc::new(privacy::CompliancePersonalData))?;
        context.on_user_deleted(Arc::new(privacy::UserDeletedListener))
    }

    fn retention(&self) -> Vec<autoxide_kernel::privacy::RetentionPolicy> {
        use autoxide_kernel::privacy::{DataCategory, Keep, RetentionPolicy};
        let callback_days = u32::try_from(self.config.callback_retention_days).unwrap_or(u32::MAX);
        vec![
            RetentionPolicy::new(
                "compliance.kyc_callbacks",
                "Raw callbacks of identity providers (identity verification results).",
                DataCategory::Sensitive,
                Keep::Days(callback_days),
            )
            .setting(config::KYC_CALLBACK_RETENTION_DAYS.name),
            RetentionPolicy::new(
                "compliance.kyc_profiles",
                "Identity verification results: verified name, document kind, last 4 digits and a \
                 hash of the document number (no image, no biometrics).",
                DataCategory::Sensitive,
                Keep::UntilDeleted,
            ),
            RetentionPolicy::new(
                "compliance.phone_verifications",
                "Verified phone number of an account; deleted with the account.",
                DataCategory::Basic,
                Keep::UntilDeleted,
            ),
            RetentionPolicy::new(
                "compliance.phone_challenges",
                "One-time codes sent by SMS (hashed), deleted after a day.",
                DataCategory::Security,
                Keep::Days(1),
            ),
            RetentionPolicy::new(
                "compliance.consents",
                "Proof of consents and of accepted legal documents, kept to prove them.",
                DataCategory::Accounting,
                Keep::Forever,
            ),
            RetentionPolicy::new(
                "compliance.cookie_consents",
                "Anonymous proof of cookie consents (random id, categories, hashed network block), \
                 kept for the days of the legal rule `privacy.consent_proof_days`.",
                DataCategory::Accounting,
                Keep::Days(1825),
            ),
            RetentionPolicy::new(
                "compliance.cases",
                "Reports of infringements and consumer complaints with the reporter's name and \
                 contact, the log and the record of disclosures to authorities, kept to answer \
                 later disputes and inspections.",
                DataCategory::Basic,
                Keep::UntilDeleted,
            ),
        ]
    }
}
