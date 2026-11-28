//! User-trackable jobs (step 51 P15, D-213): file exports, bulk operations... A module writes a
//! `kernel_job_runs` row (tenant-level, present in every data database) when it queues a job whose progress
//! the user needs to see; the worker marks it done / failed itself when the module's job finishes. Internal
//! jobs (sending mail, periodic schedules) have no row so they do not show up.
//!
//! - `GET /api/v1/kernel/jobs` - the caller's own jobs, or every job of the tenant with `kernel.jobs.read`;
//!   `GET /api/v1/kernel/jobs/{id}` - status, progress, result.
//! - `POST /api/v1/kernel/jobs/{id}/retry` (`kernel.jobs.manage`) - puts a permanently failed job back on
//!   the queue, with event `kernel.job_retried` (audit).

use crate::time::Timestamp;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, Query as SqlQuery};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{Permission, Principal};
use crate::error::{Error, FieldError};
use crate::event::{Actor, EventInfo, Outbox};
use crate::migration::prelude::*;
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;

pub(crate) const TABLE: &str = "kernel_job_runs";

/// View every trackable job of the tenant (not only one's own jobs).
pub const JOBS_READ: Permission =
    Permission::new("kernel.jobs.read", "See every tracked background job of the tenant.");
/// Re-run a failed job.
pub const JOBS_MANAGE: Permission =
    Permission::new("kernel.jobs.manage", "Run a failed background job of the tenant again.");

/// Event: a staff member re-runs a failed job.
pub(crate) const JOB_RETRIED: EventInfo =
    EventInfo::new("kernel.job_retried", "A failed background job was put back in the queue.");

#[derive(Debug, Serialize)]
struct JobRetried {
    module: String,
    kind: String,
}

impl crate::event::Event for JobRetried {
    const NAME: &'static str = "kernel.job_retried";
}

/// Records the tracking row for job `id` of module `module` that was just queued (in the same transaction
/// as `enqueue`). `label`: an English sentence for the viewer (e.g. "Export of the submissions of
/// “Contact”").
#[allow(clippy::too_many_arguments)]
pub async fn track<C: crate::db::Executor>(
    connection: &C,
    tenant: &TenantContext,
    module: &str,
    id: Uuid,
    kind: &str,
    label: &str,
    requested_by: Option<Uuid>,
    org_id: Option<Uuid>,
) -> Result<(), Error> {
    let connection = &crate::db::Via::of(connection);
    let now = crate::job::queue::now();
    let insert = SqlQuery::insert()
        .into_table(Alias::new(TABLE))
        .columns([
            "id",
            "tenant_id",
            "module",
            "kind",
            "label",
            "requested_by",
            "org_id",
            "status",
            "created_at",
            "updated_at",
        ])
        .values_panic([
            id.into(),
            tenant.id().as_uuid().into(),
            module.into(),
            kind.into(),
            label.chars().take(500).collect::<String>().into(),
            requested_by.into(),
            org_id.into(),
            "queued".into(),
            now.into(),
            now.into(),
        ])
        .to_owned();
    connection.execute(&insert).await?;
    Ok(())
}

/// The progress of job `id` (called by the job runner): `done` out of `total`.
pub async fn progress<C: crate::db::Executor>(
    connection: &C,
    id: Uuid,
    done: u64,
    total: u64,
) -> Result<(), Error> {
    let connection = &crate::db::Via::of(connection);
    let update = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(
            Alias::new("progress_done"),
            <i64 as TryFrom<u64>>::try_from(done).unwrap_or(i64::MAX),
        )
        .value(
            Alias::new("progress_total"),
            <i64 as TryFrom<u64>>::try_from(total).unwrap_or(i64::MAX),
        )
        .value(Alias::new("status"), "running")
        .value(Alias::new("updated_at"), crate::job::queue::now())
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    connection.execute(&update).await?;
    Ok(())
}

/// The result of job `id` (JSON, e.g. row count, file code, per-record results of a bulk operation).
pub async fn set_result<C: crate::db::Executor>(
    connection: &C,
    id: Uuid,
    result: &Value,
) -> Result<(), Error> {
    let connection = &crate::db::Via::of(connection);
    let update = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("result"), serde_json::to_string(result).map_err(Error::internal)?)
        .value(Alias::new("updated_at"), crate::job::queue::now())
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    connection.execute(&update).await?;
    Ok(())
}

/// The file the job just wrote (the list export, P16).
pub(crate) async fn set_file<C: ConnectionTrait>(
    connection: &C,
    id: Uuid,
    file: &JobFile,
) -> Result<(), Error> {
    let update = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("file_name"), file.name.clone())
        .value(Alias::new("file_size"), file.size)
        .value(Alias::new("file_expires_at"), file.expires_at)
        .value(Alias::new("updated_at"), crate::job::queue::now())
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    connection.execute(&update).await?;
    Ok(())
}

/// Drops a deleted file from the row of job `id`.
pub(crate) async fn clear_file<C: ConnectionTrait>(connection: &C, id: Uuid) -> Result<(), Error> {
    let update = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("file_name"), Option::<String>::None)
        .value(Alias::new("file_size"), Option::<i64>::None)
        .value(Alias::new("file_expires_at"), Option::<Timestamp>::None)
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    connection.execute(&update).await?;
    Ok(())
}

/// (tenant, job) pairs with a file expiring before `at` in this database, at most 500 per pass.
pub(crate) async fn expired_files(
    database: &crate::database::Database,
    at: Timestamp,
) -> Result<Vec<(Uuid, Uuid)>, Error> {
    let select = SqlQuery::select()
        .columns(["tenant_id", "id"])
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("file_expires_at")).lte(at))
        .limit(500)
        .to_owned();
    let rows = database.read().query_all(&select).await?;
    rows.iter().map(|row| Ok((row.try_get("", "tenant_id")?, row.try_get("", "id")?))).collect()
}

/// Event: a user-tracked job finished or failed permanently (step 51 P15/P16) - the notifications module
/// notifies the requester ("the export is done").
pub(crate) const JOB_FINISHED: EventInfo = EventInfo::new(
    "kernel.job_finished",
    "A background job someone asked for finished or failed for good.",
);

#[derive(Debug, Serialize)]
struct JobFinished {
    job_id: Uuid,
    kind: String,
    /// The English sentence of the job, e.g. "CSV export of content-entries".
    label: String,
    /// `succeeded` hay `failed`.
    status: String,
    requested_by: Uuid,
}

impl crate::event::Event for JobFinished {
    const NAME: &'static str = "kernel.job_finished";
}

/// Emits `kernel.job_finished` for job `id` of tenant `tenant`, if it is a job someone requested.
pub(crate) async fn announce(tenant: &TenantContext, id: Uuid, failed: bool) -> Result<(), Error> {
    let select = SqlQuery::select()
        .columns(["kind", "label", "requested_by"])
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .to_owned();
    let Some(row) = tenant.database().read().query_one(&select).await? else {
        return Ok(());
    };
    let Some(requested_by) = row.try_get::<Option<Uuid>>("", "requested_by")? else {
        return Ok(());
    };
    let event = JobFinished {
        job_id: id,
        kind: row.try_get("", "kind")?,
        label: row.try_get("", "label")?,
        status: if failed { "failed" } else { "succeeded" }.to_owned(),
        requested_by,
    };
    // Outbox của kernel ghi thẳng vào bảng event của kernel; bộ phát đọc nó ở lượt hỏi kế tiếp.
    let outbox = crate::event::Outbox::standalone(crate::builtin::NAME);
    let transaction = tenant.database().begin_write().await?;
    outbox
        .record(&transaction, tenant, Actor::system(), Some(format!("kernel.job:{id}")), &event)
        .await?;
    transaction.commit().await?;
    Ok(())
}

/// Called by the worker when job `id` finishes (`error` is `None`) or fails permanently; a job that is not
/// tracked has no row and the call changes nothing.
pub(crate) async fn finish<C: ConnectionTrait>(
    connection: &C,
    id: Uuid,
    error: Option<&str>,
) -> Result<(), Error> {
    let now = crate::job::queue::now();
    let update = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("status"), if error.is_some() { "failed" } else { "succeeded" })
        .value(Alias::new("error"), error.map(str::to_owned))
        .value(Alias::new("finished_at"), now)
        .value(Alias::new("updated_at"), now)
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    connection.execute(&update).await?;
    Ok(())
}

/// A tracked background job.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct JobRunView {
    pub id: Uuid,
    /// Module that runs it.
    pub module: String,
    pub kind: String,
    /// What it does, in English.
    pub label: String,
    pub requested_by: Option<Uuid>,
    pub org_id: Option<Uuid>,
    /// `queued`, `running`, `succeeded` or `failed`.
    #[schema(example = "running")]
    pub status: String,
    /// `{done, total}` once the job reports it.
    pub progress: Option<JobProgress>,
    /// What the job produced (counts, ids, per-record results…), once it reports it.
    pub result: Option<Value>,
    /// Why it failed, when `failed`.
    pub error: Option<String>,
    /// The file the job wrote (a list export), until it expires.
    pub file: Option<JobFile>,
    pub created_at: Timestamp,
    pub finished_at: Option<Timestamp>,
}

/// A file written by a job.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct JobFile {
    /// Where to download it (signed in, as the job's requester or with `kernel.jobs.read`).
    pub url: String,
    /// File name without extension.
    pub name: String,
    /// Size in bytes.
    pub size: i64,
    /// When it is deleted.
    pub expires_at: Timestamp,
}

/// How far a job is.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct JobProgress {
    pub done: i64,
    pub total: i64,
}

/// Which jobs.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct JobsQuery {
    /// Only `queued`, `running`, `succeeded` or `failed` jobs.
    status: Option<String>,
    /// Only this kind of job, e.g. `forms.export`.
    kind: Option<String>,
}

fn view(row: &sea_orm::QueryResult) -> Result<JobRunView, Error> {
    let done: Option<i64> = row.try_get("", "progress_done")?;
    let total: Option<i64> = row.try_get("", "progress_total")?;
    let result: Option<String> = row.try_get("", "result")?;
    let file_name: Option<String> = row.try_get("", "file_name")?;
    let file_size: Option<i64> = row.try_get("", "file_size")?;
    let file_expires: Option<Timestamp> = row.try_get("", "file_expires_at")?;
    let id: Uuid = row.try_get("", "id")?;
    Ok(JobRunView {
        id,
        module: row.try_get("", "module")?,
        kind: row.try_get("", "kind")?,
        label: row.try_get("", "label")?,
        requested_by: row.try_get("", "requested_by")?,
        org_id: row.try_get("", "org_id")?,
        status: row.try_get("", "status")?,
        progress: done.zip(total).map(|(done, total)| JobProgress { done, total }),
        result: result.and_then(|text| serde_json::from_str(&text).ok()),
        error: row.try_get("", "error")?,
        file: match (file_name, file_size, file_expires) {
            (Some(name), Some(size), Some(expires_at)) => Some(JobFile {
                url: format!("{}/kernel/jobs/{id}/file", crate::http::API_PREFIX),
                name,
                size,
                expires_at,
            }),
            _ => None,
        },
        created_at: row.try_get("", "created_at")?,
        finished_at: row.try_get("", "finished_at")?,
    })
}

const COLUMNS: [&str; 16] = [
    "id",
    "module",
    "kind",
    "label",
    "requested_by",
    "org_id",
    "status",
    "progress_done",
    "progress_total",
    "result",
    "error",
    "created_at",
    "finished_at",
    "file_name",
    "file_size",
    "file_expires_at",
];

#[derive(Clone)]
pub(crate) struct JobsState {
    pub outbox: Outbox,
}

pub(crate) fn routes(state: JobsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_jobs))
        .routes(routes!(get_job))
        .routes(routes!(retry_job))
        .with_state(Arc::new(state))
}

/// List background jobs.
///
/// Any signed-in user sees the jobs they started (exports, bulk changes…); with
/// `kernel.jobs.read`, every tracked job of the tenant. Newest first unless `order=asc`; paged
/// with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/jobs",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_jobs",
    security(("bearer" = [])),
    params(JobsQuery, crate::pagination::PageQuery),
    responses(
        (status = 200, description = "One page of jobs, newest first.", body = crate::pagination::Page<JobRunView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_jobs(
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<JobsQuery>,
    Query(page): Query<crate::pagination::PageQuery>,
) -> Result<crate::pagination::Page<JobRunView>, Error> {
    let mut problems = Vec::new();
    let status = query.status.as_deref().map(str::trim).filter(|status| !status.is_empty());
    if status.is_some_and(|status| !["queued", "running", "succeeded", "failed"].contains(&status))
    {
        problems.push(FieldError::new(
            "status",
            "invalid",
            "must be `queued`, `running`, `succeeded` or `failed`",
        ));
    }
    let page = crate::pagination::with_filter_problems(
        problems,
        crate::pagination::PageRequest::try_from(page),
    )?;
    let mut select = SqlQuery::select();
    select
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()));
    if principal.require(&JOBS_READ).is_err() {
        select.and_where(Expr::col(Alias::new("requested_by")).eq(principal.user_id()));
    }
    if let Some(status) = status {
        select.and_where(Expr::col(Alias::new("status")).eq(status));
    }
    if let Some(kind) = query.kind.as_deref().map(str::trim).filter(|kind| !kind.is_empty()) {
        select.and_where(Expr::col(Alias::new("kind")).eq(kind));
    }
    // Keyset theo id trong database: index (tenant_id, id), (tenant_id, requested_by, id), (tenant_id,
    // status, id), (tenant_id, kind, id).
    let rows = crate::pagination::fetch_statement_page(
        select,
        Alias::new("id"),
        |row| Ok(row.try_get("", "id")?),
        &page,
        tenant.database().read(),
    )
    .await?;
    crate::pagination::try_map_page(rows, view)
}

pub(crate) async fn find(
    tenant: &TenantContext,
    principal: &Principal,
    id: Uuid,
) -> Result<JobRunView, Error> {
    let mut select = SqlQuery::select();
    select
        .columns(COLUMNS)
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("tenant_id")).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(Alias::new("id")).eq(id));
    let row = tenant.database().read().query_one(&select).await?;
    let found = row.as_ref().map(view).transpose()?;
    // Job của người khác mà không có `kernel.jobs.read`: như không có (không lộ sự tồn tại).
    found
        .filter(|job| {
            job.requested_by == Some(principal.user_id()) || principal.require(&JOBS_READ).is_ok()
        })
        .ok_or_else(|| {
            Error::not_found("kernel.job_not_found", "There is no job `{id}`.").with_param("id", id)
        })
}

/// Get a background job.
///
/// Your own job, or any tracked job of the tenant with `kernel.jobs.read`: its status, progress
/// and result.
#[utoipa::path(
    get,
    path = "/jobs/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_job",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the job.")),
    responses(
        (status = 200, description = "The job.", body = Data<JobRunView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such job, or not yours; code `kernel.job_not_found`.", body = Problem),
    )
)]
async fn get_job(
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<JobRunView>, Error> {
    Ok(Data { data: find(&tenant, &principal, id).await? })
}

/// Run a failed background job again.
///
/// Requires `kernel.jobs.manage`. Puts a job that failed for good back in the queue, as new; the
/// retry is recorded in the audit log (event `kernel.job_retried`).
#[utoipa::path(
    post,
    path = "/jobs/{id}/retry",
    extensions(("x-permissions" = json!(["kernel.jobs.manage"]))),
    operation_id = "retry_job",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the job.")),
    responses(
        (status = 200, description = "The job, queued again.", body = Data<JobRunView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.jobs.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such job; code `kernel.job_not_found`.", body = Problem),
        (status = 409, description = "The job has not failed; code `kernel.job_not_failed`.", body = Problem),
    )
)]
async fn retry_job(
    State(state): State<Arc<JobsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<JobRunView>, Error> {
    principal.require(&JOBS_MANAGE)?;
    let job = find(&tenant, &principal, id).await?;
    let not_failed = || {
        Error::conflict(
            "kernel.job_not_failed",
            "Job `{id}` has not failed; only failed jobs run again.",
        )
        .with_param("id", id)
    };
    if job.status != "failed" {
        return Err(not_failed());
    }
    let transaction = tenant.database().begin_write().await?;
    if !crate::job::queue::retry(&transaction, &job.module, id).await? {
        transaction.rollback().await?;
        return Err(not_failed());
    }
    let reset = SqlQuery::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("status"), "queued")
        .value(Alias::new("error"), Option::<String>::None)
        .value(Alias::new("finished_at"), Option::<Timestamp>::None)
        .value(Alias::new("updated_at"), crate::job::queue::now())
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    transaction.execute(&reset).await?;
    state
        .outbox
        .record(
            &transaction,
            &tenant,
            Actor::from(&principal),
            Some(format!("kernel.job:{id}")),
            &JobRetried { module: job.module.clone(), kind: job.kind.clone() },
        )
        .await?;
    transaction.commit().await?;
    Ok(Data { data: find(&tenant, &principal, id).await? })
}

/// Migration that creates the `kernel_job_runs` table (tenant-level, every data database).
pub(crate) struct CreateJobRuns;

impl MigrationName for CreateJobRuns {
    fn name(&self) -> &str {
        "m0018_create_job_runs"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateJobRuns {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("module", 64))
                    .col(schema::string_len("kind", 128))
                    .col(schema::string_len("label", 500))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::uuid_null("org_id"))
                    .col(schema::string_len("status", 16))
                    .col(schema::big_integer_null("progress_done"))
                    .col(schema::big_integer_null("progress_total"))
                    .col(schema::text_null("result"))
                    .col(schema::text_null("error"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("kernel_job_runs_requester_idx", ["tenant_id", "requested_by", "id"]),
            ("kernel_job_runs_status_idx", ["tenant_id", "status", "id"]),
        ] {
            let mut index = Index::create();
            index.name(name).table(Alias::new(TABLE));
            for column in columns {
                index.col(Alias::new(column));
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}

/// Migration that adds the job's file columns (step 51 P16): name, size, retention deadline; an index so
/// the cleanup job can find expired files.
pub(crate) struct AddJobFiles;

impl MigrationName for AddJobFiles {
    fn name(&self) -> &str {
        "m0020_job_run_files"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddJobFiles {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm một cột mỗi lệnh `ALTER TABLE`.
        for column in [
            schema::string_len_null("file_name", 200),
            schema::big_integer_null("file_size"),
            schema::timestamp_with_time_zone_null("file_expires_at"),
        ] {
            let mut column = column;
            manager
                .alter_table(
                    Table::alter().table(Alias::new(TABLE)).add_column(&mut column).to_owned(),
                )
                .await?;
        }
        manager
            .create_index(
                Index::create()
                    .name("kernel_job_runs_file_sweep_idx")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("file_expires_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(
                Index::drop()
                    .name("kernel_job_runs_file_sweep_idx")
                    .table(Alias::new(TABLE))
                    .to_owned(),
            )
            .await?;
        for column in ["file_expires_at", "file_size", "file_name"] {
            manager
                .alter_table(
                    Table::alter()
                        .table(Alias::new(TABLE))
                        .drop_column(Alias::new(column))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
