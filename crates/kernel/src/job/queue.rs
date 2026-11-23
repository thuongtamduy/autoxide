//! Puts jobs into the queue, and operations for operators (view, rerun a failed job).
use crate::time::Timestamp;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Order, Query};
use uuid::Uuid;

use super::{Job, foreign_kind, module_of, table_name};
use crate::error::Error;
use crate::tenancy::{NO_TENANT, TenantContext, TenantId};

/// Options when putting a job into the queue. Build with `EnqueueOptions::default()` then the
/// functions that set each field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct EnqueueOptions {
    /// Run from this moment; `None`: immediately.
    pub run_at: Option<Timestamp>,
    /// Maximum number of runs; `None`: [`DEFAULT_MAX_ATTEMPTS`](super::DEFAULT_MAX_ATTEMPTS).
    pub max_attempts: Option<i32>,
    /// Deduplication key: if a job with this key already exists, no new job is added.
    pub unique_key: Option<String>,
}

impl EnqueueOptions {
    /// Runs the job no earlier than `run_at`.
    pub const fn run_at(mut self, run_at: Timestamp) -> Self {
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

/// A module's job queue. Obtained with `StartContext::jobs()`; cheap to clone.
#[derive(Debug, Clone)]
pub struct JobQueue {
    module: &'static str,
    table: String,
}

impl JobQueue {
    /// The queue of module `module`. In the program, obtain it with `StartContext::jobs()`;
    /// this function is for tests and tools.
    pub fn new(module: &'static str) -> Self {
        JobQueue { module, table: table_name(module) }
    }

    /// Puts `job` of tenant `tenant` into the queue, in `connection` (usually the transaction of the
    /// change, on `tenant.database()`). Returns the job's ID, or `None` if the tenant already has a job
    /// with the same `unique_key`.
    pub async fn enqueue<C, J>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        job: &J,
        options: EnqueueOptions,
    ) -> Result<Option<Uuid>, Error>
    where
        C: ConnectionTrait,
        J: Job,
    {
        self.enqueue_value(
            connection,
            tenant.id(),
            J::KIND,
            serde_json::to_value(job).map_err(Error::internal)?,
            options,
        )
        .await
    }

    /// Puts a database-wide `job` (belonging to no tenant - e.g. a cleanup that sweeps all tenants) into the
    /// queue of database `connection`. The runner receives [`JobContext::tenant`](super::JobContext) as
    /// `None`.
    pub async fn enqueue_database<C, J>(
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
            NO_TENANT,
            J::KIND,
            serde_json::to_value(job).map_err(Error::internal)?,
            options,
        )
        .await
    }

    pub(crate) async fn enqueue_value<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: TenantId,
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
                Alias::new("tenant_id"),
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
                tenant.as_uuid().into(),
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
            .on_conflict(
                OnConflict::columns([Alias::new("tenant_id"), Alias::new("unique_key")])
                    .do_nothing()
                    .to_owned(),
            );
        let inserted = connection.execute(&insert).await?.rows_affected();
        Ok((inserted == 1).then_some(id))
    }
}

/// The state of a job, for operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobStatus {
    /// Waiting to run or running.
    Pending,
    Completed,
    /// Permanently failed (dead-letter queue).
    Failed,
}

/// A job, for operators to view.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct JobRecord {
    pub id: Uuid,
    pub kind: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub run_at: Timestamp,
    pub failed_at: Option<Timestamp>,
    pub last_error: Option<String>,
}

impl JobRecord {
    /// Job `id` of kind `kind` due at `run_at`, not tried yet, with
    /// [`DEFAULT_MAX_ATTEMPTS`](super::DEFAULT_MAX_ATTEMPTS).
    pub fn new(id: Uuid, kind: impl Into<String>, run_at: Timestamp) -> Self {
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
    pub const fn failed_at(mut self, failed_at: Option<Timestamp>) -> Self {
        self.failed_at = failed_at;
        self
    }

    /// Sets the error of the last run.
    pub fn last_error(mut self, last_error: Option<String>) -> Self {
        self.last_error = last_error;
        self
    }
}

/// At most `limit` jobs with status `status` of module `module`, newest first.
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

/// Puts a permanently failed job back into the queue, as new. `false` if there is no failed job with this
/// ID in the module.
pub(crate) async fn retry<C: ConnectionTrait>(
    connection: &C,
    module: &str,
    id: Uuid,
) -> Result<bool, Error> {
    let mut update = Query::update();
    update
        .table(Alias::new(table_name(module)))
        .value(Alias::new("failed_at"), Option::<Timestamp>::None)
        .value(Alias::new("attempts"), 0i32)
        .value(Alias::new("run_at"), now())
        .value(Alias::new("claimed_by"), Option::<String>::None)
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .and_where(Expr::col(Alias::new("failed_at")).is_not_null());
    Ok(connection.execute(&update).await?.rows_affected() == 1)
}

pub(crate) fn now() -> Timestamp {
    Timestamp::now()
}
