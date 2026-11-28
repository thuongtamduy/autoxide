//! Job queue: background work, stored in the database (no Redis needed).
//!
//! 1. A module adds [`jobs_migration`] to its own migrations: the `<module>_jobs` table.
//! 2. A module puts a job on the queue with [`JobQueue::enqueue`] - inside the transaction of the data
//!    change if there is one, so the job exists only when the change is saved.
//! 3. A module declares the runner for each job kind (`Module::jobs`) and the periodic schedules
//!    (`Module::schedules`).
//! 4. The kernel's [`Worker`] claims jobs (the [`Claim`] trait: Postgres uses `FOR UPDATE SKIP LOCKED`,
//!    SQLite uses a single `UPDATE … RETURNING` statement - D-009), runs them and records the result. On
//!    failure it retries with backoff; beyond the allowed number of attempts the job stays with `failed_at`
//!    set (dead-letter queue), to be viewed and re-run with `autoxide jobs`.
//!
//! Multiple tenants (step 33): each job carries its tenant (`tenant_id`; a job of the whole database
//! carries [`NO_TENANT`](crate::tenancy::NO_TENANT)); the worker walks every assigned database and shares
//! turns fairly among tenants that have due jobs - one tenant cannot take over the whole queue.
//!
//! Runs **at least once**: if a process dies mid-run, the job is run again when its hold expires. Runners
//! must tolerate being run again.

pub mod backlog;
pub(crate) mod backlog_api;
mod claim;
mod migration;
pub(crate) mod queue;
pub mod runs;
mod worker;

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::database::Database;
use crate::error::Error;
use crate::tenancy::TenantContext;

pub use claim::{Claim, ClaimRequest, ClaimedJob, PostgresClaim, SqliteClaim, claim_for};
pub use migration::{jobs_migration, tenant_table};
pub use queue::{EnqueueOptions, JobQueue, JobRecord, JobStatus};
pub use worker::{DEFAULT_MAX_ATTEMPTS, Worker};

/// The content of a job: a type convertible to JSON, tied to a job kind.
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
    /// `<module>.<action>`: the module that owns the job and its runner.
    const KIND: &'static str;
}

/// A job being run. Built by the worker; tests build it with [`JobContext::for_tenant`] /
/// [`JobContext::for_database`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct JobContext {
    pub id: Uuid,
    pub kind: String,
    pub payload: Value,
    /// Which run this is (starting from 1).
    pub attempt: i32,
    /// The job's tenant; `None`: a job of the whole database (cleanup schedules,
    /// [`JobQueue::enqueue_database`]).
    pub tenant: Option<TenantContext>,
    /// The database holding the job (the tenant's, or the database a schedule job is sweeping).
    pub database: Database,
}

impl JobContext {
    /// Run number `attempt` (from 1) of a job of kind `kind` for `tenant`, with `payload`, under a
    /// new id.
    pub fn for_tenant(
        tenant: TenantContext,
        kind: impl Into<String>,
        payload: Value,
        attempt: i32,
    ) -> Self {
        let database = tenant.database().clone();
        JobContext {
            id: Uuid::now_v7(),
            kind: kind.into(),
            payload,
            attempt,
            tenant: Some(tenant),
            database,
        }
    }

    /// Run number `attempt` (from 1) of a job of the whole `database` (no tenant), of kind `kind`
    /// with `payload`, under a new id.
    pub fn for_database(
        database: Database,
        kind: impl Into<String>,
        payload: Value,
        attempt: i32,
    ) -> Self {
        JobContext {
            id: Uuid::now_v7(),
            kind: kind.into(),
            payload,
            attempt,
            tenant: None,
            database,
        }
    }

    /// Sets the job id.
    pub const fn id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    /// Handles of `db` that read and write the module's tenant records of **every** tenant on the job's
    /// database, without the tenant filter — for a job of the whole database (cleanup schedules) sweeping
    /// expired rows. `reason` says why, in a few words; each use is counted in the metric
    /// `kernel_all_tenant_queries{module}` like `ControlPlane::all_tenants`. A job of one tenant gets an internal
    /// error: it uses `tenant.db()`, so it never sees another tenant's rows on a shared database.
    pub fn all_tenants(&self, reason: &'static str) -> Result<crate::db::AllTenants<'_>, Error> {
        if self.tenant.is_some() {
            return Err(Error::internal(format!(
                "job `{}` belongs to one tenant: query through `tenant.db()`, not across every tenant",
                self.kind
            )));
        }
        // Loại job là `<module>.<việc>`: phần đầu là module (tập nhỏ cố định, dùng làm nhãn số đo được).
        let module = self.kind.split('.').next().unwrap_or_default();
        Ok(crate::db::AllTenants::counted(&self.database, module, reason))
    }
}

/// The runner of one job kind. Returning an error makes the job run again later (backoff).
#[async_trait::async_trait]
pub trait JobHandler: Send + Sync {
    async fn run(&self, job: &JobContext) -> Result<(), Error>;
}

/// The runner for job kind `kind` (`<module>.<action>`).
#[derive(Clone)]
pub struct JobRegistration {
    kind: &'static str,
    handler: Arc<dyn JobHandler>,
    timeout: Option<Duration>,
}

impl JobRegistration {
    pub fn new(kind: &'static str, handler: Arc<dyn JobHandler>) -> Self {
        JobRegistration { kind, handler, timeout: None }
    }

    /// Lets jobs of this kind run for at most `limit` (default: 4 minutes 30 seconds, under the worker's 5
    /// minute hold limit). If longer than the hold limit, the worker renews its hold on the job
    /// periodically while it runs, so other workers do not take it again and run it concurrently; if the
    /// hold is lost (e.g. returned to the queue by `autoxide jobs`) the job is stopped. For long work such
    /// as backup (step 39).
    #[must_use]
    pub fn timeout(mut self, limit: Duration) -> Self {
        self.timeout = Some(limit);
        self
    }

    pub(crate) fn time_limit(&self) -> Option<Duration> {
        self.timeout
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

/// A periodic schedule: every `every`, puts one job of kind `kind` on the queue. Even with several workers
/// running at once there is only one job per interval.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Schedule {
    /// The schedule name, unique within the module: `<module>.<name>`.
    pub name: &'static str,
    pub kind: &'static str,
    pub every: Duration,
    pub payload: Value,
    /// Only puts jobs on the queue of the control database (placement `main`), not every database: for work
    /// that touches only control-plane tables (e.g. re-checking a tenant's custom domain).
    pub control_only: bool,
}

impl Schedule {
    pub fn new(name: &'static str, kind: &'static str, every: Duration) -> Self {
        Schedule { name, kind, every, payload: Value::Null, control_only: false }
    }

    /// Runs only on the control database (see [`Schedule::control_only`]).
    #[must_use]
    pub const fn on_control_only(mut self) -> Self {
        self.control_only = true;
        self
    }
}

/// The job table name of module `module`.
pub(crate) fn table_name(module: &str) -> String {
    format!("{module}_jobs")
}

/// The module that owns job kind `kind` (the part before the first dot).
pub(crate) fn module_of(kind: &str) -> Option<&str> {
    kind.split_once('.').map(|(module, _)| module)
}

/// The error when a module uses a job kind of another module.
pub(crate) fn foreign_kind(module: &str, kind: &str) -> Error {
    Error::internal(format!(
        "module `{module}` cannot use job kind `{kind}`: job kinds of a module start with \
         `{module}.`"
    ))
}
