//! Service của `forms`: MỌI logic nằm ở đây và các file theo tính năng — `forms.rs` + `forms/`
//! (định nghĩa form, trường), `submissions/` (gửi công khai, hộp thư, CSV, mail báo),
//! `retention.rs`, `privacy.rs`. Handler REST và tool MCP chỉ lấy dữ liệu vào rồi gọi service —
//! cùng kết quả, cùng permission (luật kiến trúc 8). Mỗi lệnh ghi đi cùng event trong CÙNG
//! transaction (luật kiến trúc 3); gọi mạng (chống spam, gửi mail) nằm NGOÀI transaction.

use std::sync::Arc;

use autoxide_kernel::antispam::SpamGuard;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Outbox;
use autoxide_kernel::i18n::Translations;
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::public_url::PublicUrl;
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::storage::{Storage, TenantStorage};
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::{DbErr, SqlErr};

use crate::config::FormsConfig;

// Service không giữ database: mỗi hàm nhận `tenant` và dùng database của khách đó (bước 33).
pub(crate) struct FormsService {
    pub(crate) outbox: Outbox,
    pub(crate) guard: Arc<SpamGuard>,
    pub(crate) secret: SecretKey,
    pub(crate) mailer: Arc<dyn Mailer>,
    pub(crate) translations: Translations,
    /// Nơi lưu file xuất CSV (module `files` công bố).
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) jobs: JobQueue,
    /// Địa chỉ công khai của API, để link tải bản xuất là link tuyệt đối khi đã cấu hình.
    pub(crate) public_url: Arc<PublicUrl>,
    pub(crate) config: FormsConfig,
}

/// Thứ service cần từ kernel và module khác.
pub(crate) struct Parts {
    pub outbox: Outbox,
    pub guard: Arc<SpamGuard>,
    pub secret: SecretKey,
    pub mailer: Arc<dyn Mailer>,
    pub translations: Translations,
    pub storage: Arc<dyn Storage>,
    pub jobs: JobQueue,
    pub public_url: Arc<PublicUrl>,
}

impl FormsService {
    pub(crate) fn new(parts: Parts, config: FormsConfig) -> Self {
        FormsService {
            outbox: parts.outbox,
            guard: parts.guard,
            secret: parts.secret,
            mailer: parts.mailer,
            translations: parts.translations,
            storage: parts.storage,
            jobs: parts.jobs,
            public_url: parts.public_url,
            config,
        }
    }

    /// Nơi lưu file xuất của khách `tenant` (khoá có tiền tố khách, D-259).
    pub(crate) fn store(&self, tenant: &TenantContext) -> TenantStorage {
        TenantStorage::new(self.storage.clone(), tenant)
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
