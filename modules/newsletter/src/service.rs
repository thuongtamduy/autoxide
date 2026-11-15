//! Service của `newsletter`: MỌI logic nằm ở đây và các file theo tính năng — `audience/` (danh
//! sách, người đăng ký, chặn), `public.rs` (đăng ký công khai), `campaigns.rs` + `campaigns/`
//! (soạn, gửi, bài mới, dựng mail), `retention.rs`, `privacy.rs`. Handler REST và tool MCP chỉ
//! lấy dữ liệu vào rồi gọi service — cùng kết quả, cùng permission (luật kiến trúc 8). Mỗi lệnh
//! ghi đi cùng event trong CÙNG transaction (luật kiến trúc 3); gọi mạng (chống spam) và xếp mail
//! nằm NGOÀI transaction.

use std::sync::Arc;

use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::i18n::Translations;
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::public_url::PublicUrl;
use autoxide_kernel::resource::ResourceDirectory;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tokens::OneTimeTokens;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{DbErr, SqlErr};
use uuid::Uuid;

use crate::config::NewsletterConfig;

pub(crate) const MODULE: &str = "newsletter";

pub(crate) struct NewsletterService {
    pub(crate) database: Database,
    pub(crate) outbox: Outbox,
    pub(crate) guard: Arc<SpamGuard>,
    pub(crate) secret: SecretKey,
    pub(crate) mailer: Arc<dyn Mailer>,
    pub(crate) translations: Translations,
    pub(crate) jobs: JobQueue,
    pub(crate) tokens: OneTimeTokens,
    pub(crate) public_url: Arc<PublicUrl>,
    pub(crate) resources: ResourceDirectory,
    pub(crate) config: NewsletterConfig,
}

/// Thứ service cần từ kernel và module khác.
pub(crate) struct Parts {
    pub database: Database,
    pub outbox: Outbox,
    pub guard: Arc<SpamGuard>,
    pub secret: SecretKey,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub jobs: JobQueue,
    pub public_url: Arc<PublicUrl>,
    pub resources: ResourceDirectory,
}

impl NewsletterService {
    pub(crate) fn new(parts: Parts, config: NewsletterConfig) -> Self {
        NewsletterService {
            database: parts.database,
            outbox: parts.outbox,
            guard: parts.guard,
            secret: parts.secret,
            mailer: parts.mailer,
            translations: parts.translations,
            jobs: parts.jobs,
            tokens: OneTimeTokens::new(MODULE),
            public_url: parts.public_url,
            resources: parts.resources,
            config,
        }
    }
}

/// Bây giờ, cắt tới micro giây để hai database trả cùng giá trị.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
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
