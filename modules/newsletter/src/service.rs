//! Service của `newsletter`: MỌI logic nằm ở đây và các file theo tính năng — `audience/` (danh
//! sách, người đăng ký, chặn), `public.rs` (đăng ký công khai), `campaigns.rs` + `campaigns/`
//! (soạn, gửi, bài mới, dựng mail), `retention.rs`, `privacy.rs`. Handler REST và tool MCP chỉ
//! lấy dữ liệu vào rồi gọi service — cùng kết quả, cùng permission (luật kiến trúc 8). Mỗi lệnh
//! ghi đi cùng event trong CÙNG transaction (luật kiến trúc 3); gọi mạng (chống spam) và xếp mail
//! nằm NGOÀI transaction.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::i18n::Translations;
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::public_url::PublicUrl;
use autoxide_kernel::resource::ResourceDirectory;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::DatabaseRegistry;
use autoxide_kernel::tokens::OneTimeTokens;
use sea_orm::{DbErr, SqlErr};
use uuid::Uuid;

use crate::config::NewsletterConfig;

pub(crate) const MODULE: &str = "newsletter";

// Service không giữ database: mỗi hàm nhận `tenant` và dùng database của khách đó (bước 33).
pub(crate) struct NewsletterService {
    pub(crate) outbox: Outbox,
    pub(crate) guard: Arc<SpamGuard>,
    pub(crate) secret: SecretKey,
    pub(crate) mailer: Arc<dyn Mailer>,
    pub(crate) translations: Translations,
    pub(crate) jobs: JobQueue,
    pub(crate) tokens: OneTimeTokens,
    pub(crate) public_url: Arc<PublicUrl>,
    pub(crate) resources: ResourceDirectory,
    /// Sổ database: token trong link mang khách của nó (`autoxide_kernel::tenancy::link_token`).
    pub(crate) databases: DatabaseRegistry,
    pub(crate) config: NewsletterConfig,
    /// Link trong thư theo tên miền của khách (`kernel.public_url`); mặc định: địa chỉ máy chủ.
    pub(crate) links: autoxide_kernel::tenancy::SiteLinks,
    /// Sổ đồng ý của `compliance` (D-347, D-367); `None` khi chương trình không có `compliance`.
    pub(crate) consents: Option<Arc<dyn autoxide_compliance::api::ConsentLedger>>,
    /// Bộ dựng mẫu của kernel (chiến dịch `format = template`).
    pub(crate) templates: autoxide_kernel::template::TemplateEngine,
    /// Cấu hình theo khách (`newsletter.postal_address`); `None` trong test dựng tay.
    pub(crate) settings: Option<Arc<dyn autoxide_kernel::tenancy::TenantSettings>>,
    /// Hồ sơ pháp lý của `compliance` (địa chỉ người gửi mặc định của khách); `None` khi không cài.
    pub(crate) legal: Option<Arc<dyn autoxide_compliance::api::LegalProfiles>>,
}

/// Thứ service cần từ kernel và module khác.
pub(crate) struct Parts {
    pub outbox: Outbox,
    pub guard: Arc<SpamGuard>,
    pub secret: SecretKey,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub jobs: JobQueue,
    pub public_url: Arc<PublicUrl>,
    pub resources: ResourceDirectory,
    pub databases: DatabaseRegistry,
}

impl NewsletterService {
    pub(crate) fn new(parts: Parts, config: NewsletterConfig) -> Self {
        NewsletterService {
            outbox: parts.outbox,
            guard: parts.guard,
            secret: parts.secret,
            mailer: parts.mailer,
            translations: parts.translations,
            jobs: parts.jobs,
            tokens: OneTimeTokens::new(MODULE),
            public_url: parts.public_url,
            resources: parts.resources,
            databases: parts.databases,
            config,
            links: Default::default(),
            consents: None,
            templates: Default::default(),
            settings: None,
            legal: None,
        }
    }

    /// Dùng sổ đồng ý chung của `compliance`.
    pub(crate) fn with_consents(
        mut self,
        consents: Option<Arc<dyn autoxide_compliance::api::ConsentLedger>>,
    ) -> Self {
        self.consents = consents;
        self
    }

    /// Bộ dựng mẫu của kernel.
    pub(crate) fn with_templates(
        mut self,
        templates: autoxide_kernel::template::TemplateEngine,
    ) -> Self {
        self.templates = templates;
        self
    }

    /// Cấu hình theo khách và hồ sơ pháp lý (địa chỉ người gửi).
    pub(crate) fn with_tenant_config(
        mut self,
        settings: Option<Arc<dyn autoxide_kernel::tenancy::TenantSettings>>,
        legal: Option<Arc<dyn autoxide_compliance::api::LegalProfiles>>,
    ) -> Self {
        self.settings = settings;
        self.legal = legal;
        self
    }

    /// Link trong thư theo tên miền của khách.
    pub(crate) fn with_links(mut self, links: autoxide_kernel::tenancy::SiteLinks) -> Self {
        self.links = links;
        self
    }
}

/// Bây giờ, cắt tới micro giây để hai database trả cùng giá trị.
pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}

pub(crate) fn is_unique_violation(error: &DbErr) -> bool {
    matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_)))
}

/// Đọc cột JSON đã do chính module ghi; hỏng là lỗi nội bộ.
pub(crate) fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(Error::internal)
}

pub(crate) fn to_json<T: serde::Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(Error::internal)
}

pub(crate) fn list_not_found() -> Error {
    Error::not_found("newsletter.list_not_found", "No list has this id.")
}

pub(crate) fn subscriber_not_found() -> Error {
    Error::not_found("newsletter.subscriber_not_found", "No subscriber has this id.")
}

pub(crate) fn campaign_not_found() -> Error {
    Error::not_found("newsletter.campaign_not_found", "No campaign has this id.")
}

pub(crate) fn suppression_not_found(id: Uuid) -> Error {
    Error::not_found("newsletter.suppression_not_found", "No suppression has the id `{id}`.")
        .with_param("id", id)
}
