//! Service của `forms`: MỌI logic nằm ở đây và các file cùng cấp (`forms.rs`, `inbox.rs`,
//! `public.rs`…). Handler REST và tool MCP chỉ lấy dữ liệu vào rồi gọi service — cùng kết quả,
//! cùng permission (luật kiến trúc 8). Mỗi lệnh ghi đi cùng event trong CÙNG transaction (luật
//! kiến trúc 3); gọi mạng (chống spam, gửi mail) nằm NGOÀI transaction.

use std::sync::Arc;

use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::i18n::Translations;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::secret::SecretKey;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{DbErr, SqlErr};

use crate::config::FormsConfig;

pub(crate) struct FormsService {
    pub(crate) database: Database,
    pub(crate) outbox: Outbox,
    pub(crate) guard: Arc<SpamGuard>,
    pub(crate) secret: SecretKey,
    pub(crate) mailer: Arc<dyn Mailer>,
    pub(crate) translations: Translations,
    pub(crate) config: FormsConfig,
}

/// Thứ service cần từ kernel và module khác.
pub(crate) struct Parts {
    pub database: Database,
    pub outbox: Outbox,
    pub guard: Arc<SpamGuard>,
    pub secret: SecretKey,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
}

impl FormsService {
    pub(crate) fn new(parts: Parts, config: FormsConfig) -> Self {
        FormsService {
            database: parts.database,
            outbox: parts.outbox,
            guard: parts.guard,
            secret: parts.secret,
            mailer: parts.mailer,
            translations: parts.translations,
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

/// Số nguyên dương của cấu hình hay đầu vào sang cột `INTEGER` (i32 trên Postgres).
pub(crate) fn to_i32(value: u32) -> Result<i32, Error> {
    i32::try_from(value).map_err(Error::internal)
}

pub(crate) fn form_not_found() -> Error {
    Error::not_found("forms.not_found", "No form has this id or key.")
}

pub(crate) fn submission_not_found(id: uuid::Uuid) -> Error {
    Error::not_found("forms.submission_not_found", "No submission has the id `{id}`.")
        .with_param("id", id)
}
