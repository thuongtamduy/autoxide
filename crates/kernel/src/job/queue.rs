//! Đưa job vào hàng đợi, và các thao tác cho người vận hành (xem, chạy lại job hỏng).

use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Order, Query};
use uuid::Uuid;

use super::{Job, foreign_kind, module_of, table_name};
use crate::error::Error;

/// Tuỳ chọn khi đưa job vào hàng đợi. Dựng bằng `EnqueueOptions::default()` rồi các hàm đặt
/// từng trường.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EnqueueOptions {
    /// Chạy từ lúc này; `None`: ngay.
    pub run_at: Option<DateTime<Utc>>,
    /// Số lần chạy tối đa; `None`: [`DEFAULT_MAX_ATTEMPTS`](super::DEFAULT_MAX_ATTEMPTS).
    pub max_attempts: Option<i32>,
    /// Khoá chống trùng: đã có job mang khoá này thì không thêm job mới.
    pub unique_key: Option<String>,
}

impl EnqueueOptions {
    /// Runs the job no earlier than `run_at`.
    pub const fn run_at(mut self, run_at: DateTime<Utc>) -> Self {
        self.run_at = Some(run_at);
        self
    }

    /// Runs the job at most `max_attempts` times.
    pub const fn max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = Some(max_attempts);
        self
    }

    /// Adds no job when one with this key already exists.
    pub fn unique_key(mut self, unique_key: impl Into<String>) -> Self {
        self.unique_key = Some(unique_key.into());
        self
    }
}

/// Hàng đợi job của một module. Lấy bằng `StartContext::jobs()`; clone rẻ.
#[derive(Debug, Clone)]
pub struct JobQueue {
    module: &'static str,
    table: String,
}

impl JobQueue {
    /// Hàng đợi của module `module`. Trong chương trình, lấy bằng `StartContext::jobs()`;
    /// hàm này cho test và công cụ.
    pub fn new(module: &'static str) -> Self {
        JobQueue { module, table: table_name(module) }
    }

    /// Đưa `job` vào hàng đợi, trong `connection` (thường là transaction của thay đổi).
    /// Trả về ID của job, hoặc `None` nếu đã có job mang cùng `unique_key`.
    pub async fn enqueue<C, J>(
        &self,
        connection: &C,
        job: &J,
        options: EnqueueOptions,
    ) -> Result<Option<Uuid>, Error>
    where
        C: ConnectionTrait,
        J: Job,
    {
        self.enqueue_value(
            connection,
            J::KIND,
            serde_json::to_value(job).map_err(Error::internal)?,
            options,
        )
        .await
    }

    pub(crate) async fn enqueue_value<C: ConnectionTrait>(
        &self,
        connection: &C,
        kind: &str,
        payload: serde_json::Value,
        options: EnqueueOptions,
    ) -> Result<Option<Uuid>, Error> {
        if module_of(kind) != Some(self.module) {
            return Err(foreign_kind(self.module, kind));
        }
        let now = now();
        let id = Uuid::now_v7();
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new(&self.table))
            .columns([
                Alias::new("id"),
                Alias::new("kind"),
                Alias::new("payload"),
                Alias::new("run_at"),
                Alias::new("attempts"),
                Alias::new("max_attempts"),
                Alias::new("unique_key"),
                Alias::new("created_at"),
            ])
            .values([
                id.into(),
                kind.into(),
                payload.to_string().into(),
                options.run_at.unwrap_or(now).into(),
                0i32.into(),
                options.max_attempts.unwrap_or(super::DEFAULT_MAX_ATTEMPTS).into(),
                options.unique_key.into(),
                now.into(),
            ])
            .map_err(|error| Error::internal(format!("job insert: {error}")))?
            // Trùng `unique_key` thì không chèn — bằng `ON CONFLICT DO NOTHING`, KHÔNG để lệnh
            // lỗi rồi bắt lỗi: trong transaction, Postgres đánh dấu cả transaction là hỏng sau
            // một lệnh lỗi (lệnh sau lỗi, `COMMIT` thành `ROLLBACK`), SQLite thì không (D-158).
            .on_conflict(OnConflict::column(Alias::new("unique_key")).do_nothing().to_owned());
        let inserted = connection.execute(&insert).await?.rows_affected();
        Ok((inserted == 1).then_some(id))
    }
}

/// Trạng thái của một job, cho người vận hành.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobStatus {
    /// Chờ chạy hoặc đang chạy.
    Pending,
    Completed,
    /// Hỏng hẳn (dead-letter queue).
    Failed,
}

/// Một job, cho người vận hành xem.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct JobRecord {
    pub id: Uuid,
    pub kind: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub run_at: DateTime<Utc>,
    pub failed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

impl JobRecord {
    /// Job `id` of kind `kind` due at `run_at`, not tried yet, with
    /// [`DEFAULT_MAX_ATTEMPTS`](super::DEFAULT_MAX_ATTEMPTS).
    pub fn new(id: Uuid, kind: impl Into<String>, run_at: DateTime<Utc>) -> Self {
        JobRecord {
            id,
            kind: kind.into(),
            attempts: 0,
            max_attempts: super::DEFAULT_MAX_ATTEMPTS,
            run_at,
            failed_at: None,
            last_error: None,
        }
    }

    /// Sets how many times the job ran.
    pub const fn attempts(mut self, attempts: i32) -> Self {
        self.attempts = attempts;
        self
    }

    /// Sets the most times the job may run.
    pub const fn max_attempts(mut self, max_attempts: i32) -> Self {
        self.max_attempts = max_attempts;
        self
    }

    /// Sets when the job failed for good.
    pub const fn failed_at(mut self, failed_at: Option<DateTime<Utc>>) -> Self {
        self.failed_at = failed_at;
        self
    }

    /// Sets the error of the last run.
    pub fn last_error(mut self, last_error: Option<String>) -> Self {
        self.last_error = last_error;
        self
    }
}

/// Tối đa `limit` job trạng thái `status` của module `module`, mới nhất trước.
pub(crate) async fn list<C: ConnectionTrait>(
    connection: &C,
    module: &str,
    status: JobStatus,
    limit: u64,
) -> Result<Vec<JobRecord>, Error> {
    let mut select = Query::select();
    select
        .columns(["id", "kind", "attempts", "max_attempts", "run_at", "failed_at", "last_error"])
        .from(Alias::new(table_name(module)))
        .order_by(Alias::new("id"), Order::Desc)
        .limit(limit);
    match status {
        JobStatus::Pending => select
            .and_where(Expr::col(Alias::new("completed_at")).is_null())
            .and_where(Expr::col(Alias::new("failed_at")).is_null()),
        JobStatus::Completed => {
            select.and_where(Expr::col(Alias::new("completed_at")).is_not_null())
        }
        JobStatus::Failed => select.and_where(Expr::col(Alias::new("failed_at")).is_not_null()),
    };
    let rows = connection.query_all(&select).await?;
    rows.iter()
        .map(|row| {
            Ok(JobRecord {
                id: row.try_get("", "id")?,
                kind: row.try_get("", "kind")?,
                attempts: row.try_get("", "attempts")?,
                max_attempts: row.try_get("", "max_attempts")?,
                run_at: row.try_get("", "run_at")?,
                failed_at: row.try_get("", "failed_at")?,
                last_error: row.try_get("", "last_error")?,
            })
        })
        .collect()
}

/// Đưa một job hỏng hẳn trở lại hàng đợi, như mới. `false` nếu không có job hỏng mang ID
/// này trong module.
pub(crate) async fn retry<C: ConnectionTrait>(
    connection: &C,
    module: &str,
    id: Uuid,
) -> Result<bool, Error> {
    let mut update = Query::update();
    update
        .table(Alias::new(table_name(module)))
        .value(Alias::new("failed_at"), Option::<DateTime<Utc>>::None)
        .value(Alias::new("attempts"), 0i32)
        .value(Alias::new("run_at"), now())
        .value(Alias::new("claimed_by"), Option::<String>::None)
        .value(Alias::new("claimed_until"), Option::<DateTime<Utc>>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .and_where(Expr::col(Alias::new("failed_at")).is_not_null());
    Ok(connection.execute(&update).await?.rows_affected() == 1)
}

pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}
