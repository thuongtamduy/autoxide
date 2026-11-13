//! Job queue: việc chạy nền, lưu trong database (không cần Redis).
//!
//! 1. Module thêm [`jobs_migration`] vào migration của mình: bảng `<module>_jobs`.
//! 2. Module đưa job vào hàng đợi bằng [`JobQueue::enqueue`] — trong transaction của
//!    thay đổi dữ liệu nếu có, để job chỉ tồn tại khi thay đổi được lưu.
//! 3. Module khai bộ chạy cho từng loại job (`Module::jobs`) và lịch định kỳ
//!    (`Module::schedules`).
//! 4. [`Worker`] của kernel claim job (trait [`Claim`]: Postgres dùng
//!    `FOR UPDATE SKIP LOCKED`, SQLite dùng một câu `UPDATE … RETURNING` — D-009), chạy
//!    và ghi kết quả. Lỗi thì thử lại với backoff; quá số lần cho phép thì job nằm lại
//!    với `failed_at` (dead-letter queue), xem và chạy lại bằng `autoxide jobs`.
//!
//! Chạy **ít nhất một lần**: tiến trình chết giữa lúc chạy thì job được chạy lại khi
//! hết hạn giữ. Bộ chạy phải chịu được việc chạy lại.

mod claim;
mod migration;
mod queue;
mod worker;

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::error::Error;

pub use claim::{Claim, ClaimedJob, PostgresClaim, SqliteClaim, claim_for};
pub use migration::jobs_migration;
pub use queue::{EnqueueOptions, JobQueue, JobRecord, JobStatus};
pub use worker::{DEFAULT_MAX_ATTEMPTS, Worker};

/// Nội dung của một job: kiểu đổi được sang JSON, gắn với loại job.
///
/// ```
/// use autoxide_kernel::job::Job;
/// use serde::Serialize;
///
/// #[derive(Serialize)]
/// struct SendWelcome {
///     user_id: String,
/// }
///
/// impl Job for SendWelcome {
///     const KIND: &'static str = "mail.send_welcome";
/// }
/// ```
pub trait Job: Serialize {
    /// `<module>.<việc>`: module sở hữu job và bộ chạy của nó.
    const KIND: &'static str;
}

/// Một job đang được chạy.
#[derive(Debug, Clone, PartialEq)]
pub struct JobContext {
    pub id: Uuid,
    pub kind: String,
    pub payload: Value,
    /// Lần chạy thứ mấy (bắt đầu từ 1).
    pub attempt: i32,
}

/// Bộ chạy một loại job. Trả lỗi thì job được chạy lại sau (backoff).
#[async_trait::async_trait]
pub trait JobHandler: Send + Sync {
    async fn run(&self, job: &JobContext) -> Result<(), Error>;
}

/// Bộ chạy cho loại job `kind` (`<module>.<việc>`).
#[derive(Clone)]
pub struct JobRegistration {
    kind: &'static str,
    handler: Arc<dyn JobHandler>,
}

impl JobRegistration {
    pub fn new(kind: &'static str, handler: Arc<dyn JobHandler>) -> Self {
        JobRegistration { kind, handler }
    }

    pub fn kind(&self) -> &'static str {
        self.kind
    }

    pub(crate) fn handler(&self) -> Arc<dyn JobHandler> {
        Arc::clone(&self.handler)
    }
}

impl std::fmt::Debug for JobRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobRegistration").field("kind", &self.kind).finish_non_exhaustive()
    }
}

/// Lịch định kỳ: cứ mỗi `every`, đưa một job loại `kind` vào hàng đợi. Nhiều worker
/// chạy cùng lúc vẫn chỉ có một job cho mỗi khoảng.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Schedule {
    /// Tên lịch, duy nhất trong module: `<module>.<tên>`.
    pub name: &'static str,
    pub kind: &'static str,
    pub every: Duration,
    pub payload: Value,
}

impl Schedule {
    pub fn new(name: &'static str, kind: &'static str, every: Duration) -> Self {
        Schedule { name, kind, every, payload: Value::Null }
    }
}

/// Tên bảng job của module `module`.
pub(crate) fn table_name(module: &str) -> String {
    format!("{module}_jobs")
}

/// Module sở hữu loại job `kind` (phần trước dấu chấm đầu tiên).
pub(crate) fn module_of(kind: &str) -> Option<&str> {
    kind.split_once('.').map(|(module, _)| module)
}

/// Lỗi khi một module dùng loại job của module khác.
pub(crate) fn foreign_kind(module: &str, kind: &str) -> Error {
    Error::internal(format!(
        "module `{module}` cannot use job kind `{kind}`: job kinds of a module start with \
         `{module}.`"
    ))
}
