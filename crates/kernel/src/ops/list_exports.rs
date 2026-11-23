//! Export a list by filter (step 51 P16, D-213).
//!
//! A module declares an exportable list with [`StartContext::export_source`](crate::module::StartContext) —
//! `resource` equals the P7 list id (`x-autoxide-list`), the columns, the permission, and a paged read
//! function. `POST /api/v1/kernel/exports {resource, params, columns}` validates `params` against the P7
//! filter schema, enqueues the job `kernel.export_list` (followed through the P15 job API) and returns 201
//! pointing to the job. The job reads page by page **with the requester's permissions** (a
//! `PrincipalSnapshot`), and writes CSV that is safe against CSV injection ([`crate::csv`]) into the tenant's
//! storage (`jobs/<id>.csv`). Download with `GET /api/v1/kernel/jobs/{id}/file`; the file is kept for
//! `AUTOXIDE_JOBS_FILE_RETENTION_DAYS` days and then a daily job deletes it.

use crate::time::Timestamp;
use jiff::SignedDuration;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{Permission, Principal, PrincipalSnapshot};
use crate::config::Setting;
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::http::ValidJson;
use crate::job::runs::{self, JobRunView};
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use crate::ops::views::ListSchemas;
use crate::response::{Created, Problem};
use crate::storage::{Storage, TenantStorage};
use crate::tenancy::{DatabaseRegistry, TenantContext, TenantId};

/// Job type that writes the export file.
pub(crate) const EXPORT_LIST: &str = "kernel.export_list";
/// Job type that cleans up expired export files (daily, per data database).
pub(crate) const CLEAN_FILES: &str = "kernel.clean_job_files";
/// At most this many rows per export; longer ones are cut and marked `truncated`.
const MAX_ROWS: u64 = 100_000;

pub const FILE_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_JOBS_FILE_RETENTION_DAYS",
    "Days the file of a background job (a list export) is kept for download (1 to 365).",
)
.with_default("7");

/// Declaration of one exportable list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportInfo {
    /// List id, as the endpoint's `x-autoxide-list` (`<module>.<operation id>`).
    pub resource: &'static str,
    /// File name (without extension), e.g. `content-entries`.
    pub file_name: &'static str,
    /// All columns in default order; [`ExportPage::rows`] follows exactly this order.
    pub columns: Vec<&'static str>,
    /// Record type that has custom fields (P19): the export can add `cf.<key>` columns for the enabled fields,
    /// with values taken from [`ExportPage::custom`].
    pub custom_fields: Option<&'static str>,
}

impl ExportInfo {
    pub fn new(
        resource: &'static str,
        file_name: &'static str,
        columns: Vec<&'static str>,
    ) -> Self {
        ExportInfo { resource, file_name, columns, custom_fields: None }
    }

    /// Records of the list that carry custom fields of type `target` (e.g. `content.entry`).
    pub fn with_custom_fields(mut self, target: &'static str) -> Self {
        self.custom_fields = Some(target);
        self
    }
}

/// Filters the user sent (validated against the P7 schema), read into the list's `IntoParams` type.
#[derive(Debug, Clone, Default)]
pub struct ExportQuery {
    params: serde_json::Map<String, Value>,
}

impl ExportQuery {
    pub fn new(params: serde_json::Map<String, Value>) -> Self {
        ExportQuery { params }
    }

    /// A copy with the parameter `name` added (or replaced) — e.g. `limit`, `cursor` when reading page by page.
    pub fn with(&self, name: &str, value: impl Into<String>) -> Self {
        let mut params = self.params.clone();
        params.insert(name.to_owned(), Value::String(value.into()));
        ExportQuery { params }
    }

    /// Reads the filters into `T` the way axum reads the query string of the list endpoint itself — the same
    /// `IntoParams` type, the same interpretation of values.
    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, Error> {
        let query: Vec<String> = self
            .params
            .iter()
            .map(|(name, value)| {
                let text = match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                format!("{}={}", encode(name), encode(&text))
            })
            .collect();
        let uri: axum::http::Uri = format!("/?{}", query.join("&"))
            .parse()
            .map_err(|error| Error::internal(format!("export query: {error}")))?;
        axum::extract::Query::<T>::try_from_uri(&uri)
            .map(|query| query.0)
            .map_err(|error| Error::internal(format!("export query: {error}")))
    }
}

/// Percent-encoding for a query string: keeps digits, ASCII letters and `-._~`, and writes every other UTF-8
/// byte as `%XX` (RFC 3986).
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// One page of the export.
#[derive(Debug, Clone, Default)]
pub struct ExportPage {
    /// Each row has every column of [`ExportInfo::columns`], in the same order.
    pub rows: Vec<Vec<String>>,
    /// Custom fields of each row (same order as `rows`), when the list has them.
    pub custom: Vec<serde_json::Map<String, Value>>,
    /// Cursor of the next page; `None`: no more.
    pub next: Option<String>,
}

impl ExportPage {
    /// A page built from records already converted to JSON as the list endpoint returns them: each column is
    /// a top-level field of the same name (strings kept as is, `null` / missing become an empty cell,
    /// anything else is written as JSON). The record's `custom_fields` field (an object), if present, becomes
    /// [`custom`](Self::custom).
    pub fn from_json<T: Serialize>(
        items: &[T],
        columns: &[&str],
        next: Option<String>,
    ) -> Result<Self, Error> {
        let mut rows = Vec::with_capacity(items.len());
        let mut custom = Vec::with_capacity(items.len());
        for item in items {
            let value = serde_json::to_value(item).map_err(Error::internal)?;
            rows.push(columns.iter().map(|column| cell(value.get(column))).collect());
            custom.push(
                value.get("custom_fields").and_then(Value::as_object).cloned().unwrap_or_default(),
            );
        }
        Ok(ExportPage { rows, custom, next })
    }
}

/// CSV cell for a JSON value: strings kept as is, `null` / missing become an empty cell, anything else is written as JSON.
fn cell(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// An exportable list.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait ExportSource: Send + Sync {
    fn info(&self) -> ExportInfo;
    /// Permission required to export (usually the permission that reads the list).
    fn permission(&self) -> Permission;
    /// The page after `cursor` (`None`: first page), read with `principal`'s permissions like the list
    /// endpoint.
    async fn page(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        query: &ExportQuery,
        cursor: Option<String>,
    ) -> Result<ExportPage, Error>;
}

type Entry = (&'static str, Arc<dyn ExportSource>);

/// Registry of exportable lists of all modules. Cheap to clone.
#[derive(Clone, Default)]
pub struct ExportSources {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl ExportSources {
    pub(crate) fn register(
        &self,
        module: &'static str,
        source: Arc<dyn ExportSource>,
    ) -> Result<(), Error> {
        let info = source.info();
        if !info.resource.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "export `{}` of module `{module}` must be named `{module}.<list>`",
                info.resource
            )));
        }
        if info.columns.is_empty() {
            return Err(Error::internal(format!("export `{}` has no column", info.resource)));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.info().resource == info.resource) {
            return Err(Error::internal(format!("export `{}` is registered twice", info.resource)));
        }
        entries.push((module, source));
        Ok(())
    }

    pub(crate) fn find(&self, resource: &str) -> Option<Entry> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(_, source)| source.info().resource == resource)
            .cloned()
    }
}

/// Holder of the list schema registry: `Modules::routes` fills it after building the routes of all modules.
#[derive(Clone, Default)]
pub(crate) struct ListsHandle(Arc<OnceLock<ListSchemas>>);

impl ListsHandle {
    pub(crate) fn fill(&self, lists: ListSchemas) {
        let _ = self.0.set(lists);
    }

    pub(crate) fn get(&self) -> ListSchemas {
        self.0.get().cloned().unwrap_or_default()
    }
}

/// Job payload: the filters and a snapshot of the requester (no list data).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExportJob {
    resource: String,
    params: serde_json::Map<String, Value>,
    columns: Vec<String>,
    principal: PrincipalSnapshot,
}

impl Job for ExportJob {
    const KIND: &'static str = EXPORT_LIST;
}

/// A list to export.
#[derive(Debug, Clone, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewExport {
    /// The list, as `x-autoxide-list` of its endpoint, e.g. `content.list_content_entries`.
    pub resource: String,
    /// Filters and order of the list, checked like a saved view (see `x-autoxide-filters`).
    #[serde(default)]
    #[schema(value_type = Object)]
    pub params: serde_json::Map<String, Value>,
    /// The columns to write, in order; omitted: every column of the export.
    #[serde(default)]
    pub columns: Vec<String>,
    /// Only `csv`, the default.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ExportsState {
    pub sources: ExportSources,
    pub lists: ListsHandle,
    pub jobs: JobQueue,
    pub entitlements: EntitlementsHandle,
    pub registry: DatabaseRegistry,
    pub fields: crate::ops::custom_fields::CustomFields,
}

fn storage(registry: &DatabaseRegistry, tenant: &TenantContext) -> Result<TenantStorage, Error> {
    let shared = registry.shared_storage().ok_or_else(|| {
        Error::unavailable(
            "kernel.export_storage_unavailable",
            "Exports need file storage; the `files` module is not installed.",
        )
    })?;
    Ok(TenantStorage::new(shared, tenant))
}

fn key(id: Uuid) -> String {
    format!("jobs/{id}.csv")
}

fn now() -> Timestamp {
    Timestamp::now()
}

/// Export a list.
///
/// Requires the permission of the list's export (usually the one that reads the list). Exports
/// the records of `resource` matching `params` as CSV in a background job: follow it with
/// `GET /api/v1/kernel/jobs/{id}` and download the file from `GET /api/v1/kernel/jobs/{id}/file`
/// once `succeeded`. The job reads with the caller's permissions; at most 100 000 rows
/// (`result.truncated` says when more matched).
#[utoipa::path(
    post,
    path = "/exports",
    extensions(("x-permissions" = json!([]))),
    operation_id = "export_list",
    security(("bearer" = [])),
    request_body = NewExport,
    responses(
        (status = 201, description = "The export job, queued.", body = crate::response::Data<JobRunView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing the permission of the export; code `permission_denied`.", body = Problem),
        (status = 422, description = "Unknown list, filter, column or format; see `errors`.", body = Problem),
        (status = 503, description = "No file storage; code `kernel.export_storage_unavailable`.", body = Problem),
    )
)]
async fn export_list(
    State(state): State<Arc<ExportsState>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<NewExport>,
) -> Result<Created<JobRunView>, Error> {
    let mut problems = Vec::new();
    if input.format.as_deref().is_some_and(|format| format != "csv") {
        problems.push(FieldError::new("format", "invalid", "must be `csv`"));
    }
    let found = state.sources.find(&input.resource);
    let enabled = match &found {
        Some((module, _)) => state.entitlements.module_enabled(tenant.id(), module).await?,
        None => false,
    };
    let Some((_, source)) = found.filter(|_| enabled) else {
        problems.push(FieldError::new(
            "resource",
            "unknown",
            "is not a list that can be exported on this server",
        ));
        return Err(Error::validation(problems));
    };
    let info = source.info();
    match state.lists.get().parameters(info.resource) {
        Some(parameters) => {
            crate::ops::views::check_params(parameters, &input.params, &mut problems)
        }
        None if input.params.is_empty() => {}
        None => problems.push(FieldError::new("params", "invalid", "this list takes no filter")),
    }
    // Cột của danh sách và cột `cf.<khoá>` của các trường tự định nghĩa đang bật (P19).
    let mut known: Vec<String> = info.columns.iter().map(|column| (*column).to_owned()).collect();
    if let Some(target) = info.custom_fields {
        for field in state.fields.definitions(&tenant, target).await? {
            known.push(format!("cf.{}", field.key));
        }
    }
    let columns = if input.columns.is_empty() { known.clone() } else { input.columns.clone() };
    if columns.len() > 100 || !columns.iter().all(|column| known.contains(column)) {
        problems.push(
            FieldError::new("columns", "invalid", "may only name columns of the export: {columns}")
                .with_param("columns", known.join(", ")),
        );
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    principal.require(&source.permission())?;
    // Không có nơi lưu thì báo ngay, không xếp job chắc chắn hỏng.
    storage(&state.registry, &tenant)?;
    let job = ExportJob {
        resource: input.resource.clone(),
        params: input.params,
        columns,
        principal: principal.snapshot(),
    };
    let transaction = tenant.database().begin_write().await?;
    let id = state
        .jobs
        .enqueue(&transaction, &tenant, &job, EnqueueOptions::default().max_attempts(3))
        .await?
        .ok_or_else(|| Error::internal("the export job was not queued"))?;
    runs::track(
        &transaction,
        &tenant,
        crate::builtin::NAME,
        id,
        EXPORT_LIST,
        &format!("CSV export of {}", info.file_name),
        Some(principal.user_id()),
        None,
    )
    .await?;
    transaction.commit().await?;
    let view = runs::find(&tenant, &principal, id).await?;
    Ok(Created::new(format!("/api/v1/kernel/jobs/{id}"), view))
}

/// Download the file of a job.
///
/// Your own job, or any job of the tenant with `kernel.jobs.read`. The CSV file of a finished
/// export, until `file.expires_at`.
#[utoipa::path(
    get,
    path = "/jobs/{id}/file",
    extensions(("x-permissions" = json!([]))),
    operation_id = "download_job_file",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the job.")),
    responses(
        (status = 200, description = "The file.", content_type = "text/csv"),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such job (code `kernel.job_not_found`), or it has no file (any more): code `kernel.job_file_not_found`.", body = Problem),
    )
)]
async fn download_job_file(
    State(state): State<Arc<ExportsState>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    let job = runs::find(&tenant, &principal, id).await?;
    let missing = || {
        Error::not_found("kernel.job_file_not_found", "Job `{id}` has no file to download.")
            .with_param("id", id)
    };
    let file = job.file.filter(|file| file.expires_at > now()).ok_or_else(missing)?;
    let stored = storage(&state.registry, &tenant)?.get(&key(id)).await?.ok_or_else(missing)?;
    let mut response = Body::from_stream(stored.content).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(stored.size));
    let disposition = crate::http::content_disposition(
        crate::http::Disposition::Attachment,
        &format!("{}.csv", file.name),
    );
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

pub(crate) fn routes(state: ExportsState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(export_list))
        .routes(routes!(download_job_file))
        .with_state(Arc::new(state))
}

/// Runner of the job `kernel.export_list`.
pub(crate) struct ExportRunner {
    pub sources: ExportSources,
    pub registry: DatabaseRegistry,
    pub retention_days: u32,
}

#[async_trait::async_trait]
impl JobHandler for ExportRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant =
            job.tenant.clone().ok_or_else(|| Error::internal("a list export runs for a tenant"))?;
        let payload: ExportJob = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("export job payload: {error}")))?;
        let (_, source) = self
            .sources
            .find(&payload.resource)
            .ok_or_else(|| Error::internal(format!("no export `{}`", payload.resource)))?;
        let info = source.info();
        // Mỗi cột: vị trí trong dòng của nguồn, hay khoá của trường tự định nghĩa (`cf.<khoá>`).
        let picks: Vec<Result<usize, String>> = payload
            .columns
            .iter()
            .filter_map(|column| match column.strip_prefix("cf.") {
                Some(key) if info.custom_fields.is_some() => Some(Err(key.to_owned())),
                _ => info.columns.iter().position(|known| known == column).map(Ok),
            })
            .collect();
        let principal = payload.principal.restore();
        let query = ExportQuery::new(payload.params);
        let store = storage(&self.registry, &tenant)?;
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, Error>>(8);
        // `unfold`: biến đầu nhận của kênh thành luồng byte cho nơi lưu đọc dần.
        let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        });
        let id = job.id;
        let database = tenant.database().clone();
        let header: Vec<String> = payload.columns.clone();
        let producer = async {
            let mut text = String::from(crate::csv::BOM);
            text.push_str(&crate::csv::line(&header));
            sender.send(Ok(bytes::Bytes::from(text))).await.map_err(Error::internal)?;
            let mut cursor = None;
            let mut written: u64 = 0;
            let mut truncated = false;
            loop {
                let page = source.page(&tenant, &principal, &query, cursor).await?;
                let mut chunk = String::new();
                for (index, row) in page.rows.iter().enumerate() {
                    if written >= MAX_ROWS {
                        truncated = true;
                        break;
                    }
                    let custom = page.custom.get(index);
                    let picked: Vec<String> = picks
                        .iter()
                        .map(|pick| match pick {
                            Ok(position) => row.get(*position).cloned().unwrap_or_default(),
                            Err(key) => cell(custom.and_then(|custom| custom.get(key))),
                        })
                        .collect();
                    chunk.push_str(&crate::csv::line(&picked));
                    written += 1;
                }
                sender.send(Ok(bytes::Bytes::from(chunk))).await.map_err(Error::internal)?;
                runs::progress(database.write(), id, written, written).await?;
                match page.next {
                    Some(next) if !truncated => cursor = Some(next),
                    _ => break,
                }
            }
            // Đóng kênh: nơi lưu biết đã hết nội dung.
            drop(sender);
            Ok::<_, Error>((written, truncated))
        };
        let file_key = key(id);
        let (stored, produced) = tokio::join!(store.put(&file_key, Box::pin(stream)), producer);
        let (rows, truncated) = produced?;
        let size = stored?;
        let expires_at = now() + SignedDuration::from_hours(24 * (i64::from(self.retention_days)));
        let file = runs::JobFile {
            url: format!("{}/kernel/jobs/{id}/file", crate::http::API_PREFIX),
            name: info.file_name.to_owned(),
            size: i64::try_from(size).unwrap_or(i64::MAX),
            expires_at,
        };
        runs::set_file(database.write(), id, &file).await?;
        runs::set_result(
            database.write(),
            id,
            &serde_json::json!({ "rows": rows, "truncated": truncated }),
        )
        .await
    }
}

/// Runner of the job that cleans up expired files of one data database.
pub(crate) struct CleanRunner {
    pub registry: DatabaseRegistry,
}

#[async_trait::async_trait]
impl JobHandler for CleanRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let due = runs::expired_files(&job.database, now()).await?;
        for (tenant_id, id) in due {
            let tenant = self.registry.tenant(TenantId::from_uuid(tenant_id)).await?;
            if let Err(error) = storage(&self.registry, &tenant)?.delete(&key(id)).await {
                tracing::warn!(job_id = %id, error = %error, "cannot delete an expired job file");
                continue;
            }
            runs::clear_file(job.database.write(), id).await?;
        }
        Ok(())
    }
}

/// Daily cleanup schedule, on every data database.
pub(crate) fn schedule() -> crate::job::Schedule {
    crate::job::Schedule::new(CLEAN_FILES, CLEAN_FILES, std::time::Duration::from_secs(86_400))
}
