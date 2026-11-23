//! Bulk operations (step 51 P8, D-201).
//!
//! A module declares actions with [`StartContext::bulk_action`](crate::module::StartContext): a list
//! (`resource`, the id from P7), the action name, the permission, and a function that works on ONE record.
//! `POST /api/v1/kernel/bulk/{resource}/{action}` accepts **one of two**: `ids` (at most 500) or `filter` +
//! `expected_count` (the list's filter; the kernel counts with the P16 export source; if the matching count
//! differs from `expected_count` it returns 409 `kernel.bulk_count_changed` and the user reviews before doing
//! it). `Idempotency-Key` is required (the kernel's idempotency layer returns the first result when called
//! again).
//!
//! The work runs as the job `kernel.bulk` (tracked through P15) with the requester's permissions; each record
//! is permission-checked separately inside the module's function. The result per record (`ok`, `skipped` with a
//! reason, `failed` with a `code`) is in the job's `result`. The module's function must tolerate being redone
//! (the job reruns after a mid-way failure): a record already in the target state returns `skipped`.

use std::sync::{Arc, PoisonError, RwLock};

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::{Permission, Principal, PrincipalSnapshot};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::http::ValidJson;
use crate::job::runs::{self, JobRunView};
use crate::job::{EnqueueOptions, Job, JobContext, JobHandler, JobQueue};
use crate::ops::list_exports::{ExportQuery, ExportSources, ListsHandle};
use crate::response::{Created, Data, Problem};
use crate::tenancy::TenantContext;

/// Job kind that runs bulk operations.
pub(crate) const BULK: &str = "kernel.bulk";
/// At most this many ids sent directly.
const MAX_IDS: usize = 500;
/// At most this many records selected by a filter.
const MAX_MATCHED: usize = 10_000;

/// Declaration part of a bulk action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BulkInfo {
    /// List id (`x-autoxide-list`), e.g. `content.list_content_entries`.
    pub resource: &'static str,
    /// Action name, lowercase letters and `_`, e.g. `archive`.
    pub action: &'static str,
    /// English sentence for people viewing the job, e.g. "Archive entries".
    pub label: &'static str,
}

impl BulkInfo {
    pub const fn new(resource: &'static str, action: &'static str, label: &'static str) -> Self {
        BulkInfo { resource, action, label }
    }
}

/// Result on one record (an error returns `Err`: the result is `failed` with the error's `code`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkOutcome {
    Done,
    /// Not done, with a stable reason (e.g. `already_archived`).
    Skipped(&'static str),
}

/// A bulk action.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait BulkAction: Send + Sync {
    fn info(&self) -> BulkInfo;
    /// Permission the requester must have (checked on receiving the request; the `apply` function still checks each record).
    fn permission(&self) -> Permission;
    async fn apply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<BulkOutcome, Error>;
}

type Entry = (&'static str, Arc<dyn BulkAction>);

/// Registry of every module's bulk actions. Cheap to clone.
#[derive(Clone, Default)]
pub struct BulkActions {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl BulkActions {
    pub(crate) fn register(
        &self,
        module: &'static str,
        action: Arc<dyn BulkAction>,
    ) -> Result<(), Error> {
        let info = action.info();
        if !info.resource.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "bulk action `{}` of module `{module}` must act on a list `{module}.<list>`",
                info.action
            )));
        }
        if info.action.is_empty()
            || !info
                .action
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(Error::internal(format!(
                "bulk action `{}` must be lower-case letters, digits and `_`",
                info.action
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| {
            let known = known.info();
            known.resource == info.resource && known.action == info.action
        }) {
            return Err(Error::internal(format!(
                "bulk action `{}` of `{}` is registered twice",
                info.action, info.resource
            )));
        }
        entries.push((module, action));
        Ok(())
    }

    fn find(&self, resource: &str, action: &str) -> Option<Entry> {
        self.entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(_, known)| {
                let info = known.info();
                info.resource == resource && info.action == action
            })
            .cloned()
    }
}

/// Job payload: the selected ids and a snapshot of the requester.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BulkJob {
    resource: String,
    action: String,
    ids: Vec<String>,
    principal: PrincipalSnapshot,
}

impl Job for BulkJob {
    const KIND: &'static str = BULK;
}

/// Records to act on: either `ids`, or `filter` with `expected_count`.
#[derive(Debug, Clone, Default, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
pub(crate) struct BulkRequest {
    /// Up to 500 record ids.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// Filters of the list (see `x-autoxide-filters`): every matching record.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub filter: Option<serde_json::Map<String, Value>>,
    /// How many records the caller saw matching `filter`; required with it, unless `dry_run`.
    #[serde(default)]
    pub expected_count: Option<u64>,
    /// `true`: only say which records would be acted on (`200` with `count` and `ids`); nothing is
    /// queued or changed.
    #[serde(default)]
    pub dry_run: bool,
}

/// What a bulk request would act on.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct BulkPreview {
    pub count: u64,
    pub ids: Vec<String>,
}

/// What happened to one record.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct BulkItem {
    pub id: String,
    /// `ok`, `skipped` or `failed`.
    pub status: String,
    /// Why it was skipped or failed (an error `code`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

#[derive(Clone)]
pub(crate) struct BulkState {
    pub actions: BulkActions,
    pub sources: ExportSources,
    pub lists: ListsHandle,
    pub jobs: JobQueue,
    pub entitlements: EntitlementsHandle,
}

/// Run an action on many records.
///
/// Requires the permission of the action, and an `Idempotency-Key`. Send either `ids` (up to 500)
/// or `filter` with `expected_count`: when the records matching `filter` now are not
/// `expected_count`, nothing is done (409 `kernel.bulk_count_changed`). The action runs in a
/// background job with your permissions, checked on each record: follow it with
/// `GET /api/v1/kernel/jobs/{id}`; its `result.items` says what happened to each record (`ok`,
/// `skipped` with a reason, `failed` with the error `code`). With `dry_run`, nothing is done: the
/// answer lists the records it would act on.
#[utoipa::path(
    post,
    path = "/bulk/{resource}/{action}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "run_bulk_action",
    security(("bearer" = [])),
    params(
        ("resource" = String, Path, description = "The list, as `x-autoxide-list`, e.g. `content.list_content_entries`."),
        ("action" = String, Path, description = "The action, e.g. `archive`."),
        ("Idempotency-Key" = String, Header, description = "Required: a random UUID per bulk request."),
    ),
    request_body = BulkRequest,
    responses(
        (status = 200, description = "With `dry_run`: the records it would act on.", body = Data<BulkPreview>),
        (status = 201, description = "The job, queued.", body = Data<JobRunView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing the permission of the action; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such action on this list; code `kernel.bulk_action_not_found`.", body = Problem),
        (status = 409, description = "The records matching `filter` are not `expected_count` any more; code `kernel.bulk_count_changed`.", body = Problem),
        (status = 422, description = "Invalid request (both or neither of `ids` and `filter`, too many records, missing `Idempotency-Key`); see `errors`.", body = Problem),
    )
)]
async fn run_bulk_action(
    State(state): State<Arc<BulkState>>,
    tenant: TenantContext,
    principal: Principal,
    Path((resource, action)): Path<(String, String)>,
    headers: HeaderMap,
    ValidJson(input): ValidJson<BulkRequest>,
) -> Result<Response, Error> {
    let not_found = || {
        Error::not_found(
            "kernel.bulk_action_not_found",
            "There is no bulk action `{action}` on `{resource}`.",
        )
        .with_param("action", action.clone())
        .with_param("resource", resource.clone())
    };
    let (module, handler) = state.actions.find(&resource, &action).ok_or_else(not_found)?;
    if !state.entitlements.module_enabled(tenant.id(), module).await? {
        return Err(not_found());
    }
    let mut problems = Vec::new();
    if !input.dry_run && !headers.contains_key("idempotency-key") {
        problems.push(FieldError::new("Idempotency-Key", "required", "is required"));
    }
    match (&input.ids, &input.filter) {
        (Some(_), Some(_)) | (None, None) => problems.push(FieldError::new(
            "ids",
            "invalid",
            "send either `ids` or `filter`, not both",
        )),
        (Some(ids), None) if ids.is_empty() || ids.len() > MAX_IDS => problems.push(
            FieldError::new("ids", "invalid", "must hold 1 to {max} ids")
                .with_param("max", MAX_IDS),
        ),
        (None, Some(filter)) => {
            if input.expected_count.is_none() && !input.dry_run {
                problems.push(FieldError::new("expected_count", "required", "is required"));
            }
            match state.lists.get().parameters(&resource) {
                Some(parameters) => {
                    crate::ops::views::check_params(parameters, filter, &mut problems)
                }
                None => problems.push(FieldError::new(
                    "filter",
                    "invalid",
                    "this list cannot be selected by filter; send `ids`",
                )),
            }
        }
        _ => {}
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    principal.require(&handler.permission())?;
    let ids = match (input.ids, input.filter) {
        (Some(ids), _) => ids,
        (None, Some(filter)) => {
            let ids = matching_ids(&state, &tenant, &principal, &resource, filter).await?;
            let found = u64::try_from(ids.len()).unwrap_or(u64::MAX);
            if input.expected_count.is_some_and(|expected| expected != found) {
                let expected = input.expected_count.unwrap_or_default();
                return Err(Error::conflict(
                    "kernel.bulk_count_changed",
                    "{found} records match the filter now, not {expected}; review the list and try \
                     again.",
                )
                .with_param("found", ids.len())
                .with_param("expected", expected));
            }
            ids
        }
        (None, None) => Vec::new(),
    };
    if input.dry_run {
        let count = u64::try_from(ids.len()).unwrap_or(u64::MAX);
        return Ok(Data { data: BulkPreview { count, ids } }.into_response());
    }
    let info = handler.info();
    let job = BulkJob {
        resource: resource.clone(),
        action: action.clone(),
        ids,
        principal: principal.snapshot(),
    };
    let count = job.ids.len();
    let transaction = tenant.database().begin_write().await?;
    let id = state
        .jobs
        .enqueue(&transaction, &tenant, &job, EnqueueOptions::default().max_attempts(3))
        .await?
        .ok_or_else(|| Error::internal("the bulk job was not queued"))?;
    runs::track(
        &transaction,
        &tenant,
        crate::builtin::NAME,
        id,
        BULK,
        &format!("{} ({count} records)", info.label),
        Some(principal.user_id()),
        None,
    )
    .await?;
    transaction.commit().await?;
    let view = runs::find(&tenant, &principal, id).await?;
    Ok(Created::new(format!("/api/v1/kernel/jobs/{id}"), view).into_response())
}

/// Ids of every record matching the filter, read with the list's export source (column `id`), with the caller's permissions.
async fn matching_ids(
    state: &BulkState,
    tenant: &TenantContext,
    principal: &Principal,
    resource: &str,
    filter: serde_json::Map<String, Value>,
) -> Result<Vec<String>, Error> {
    let source = state
        .sources
        .find(resource)
        .map(|(_, source)| source)
        .filter(|source| source.info().columns.contains(&"id"))
        .ok_or_else(|| {
            Error::validation(vec![FieldError::new(
                "filter",
                "invalid",
                "this list cannot be selected by filter; send `ids`",
            )])
        })?;
    let position = source.info().columns.iter().position(|column| *column == "id").unwrap_or(0);
    let query = ExportQuery::new(filter);
    let mut ids = Vec::new();
    let mut cursor = None;
    loop {
        let page = source.page(tenant, principal, &query, cursor).await?;
        ids.extend(page.rows.into_iter().filter_map(|row| row.into_iter().nth(position)));
        if ids.len() > MAX_MATCHED {
            return Err(Error::validation(vec![
                FieldError::new("filter", "invalid", "matches more than {max} records; narrow it")
                    .with_param("max", MAX_MATCHED),
            ]));
        }
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(ids),
        }
    }
}

pub(crate) fn routes(state: BulkState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(run_bulk_action)).with_state(Arc::new(state))
}

/// Runner of the `kernel.bulk` job.
pub(crate) struct BulkRunner {
    pub actions: BulkActions,
}

#[async_trait::async_trait]
impl JobHandler for BulkRunner {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant =
            job.tenant.clone().ok_or_else(|| Error::internal("a bulk action runs for a tenant"))?;
        let payload: BulkJob = serde_json::from_value(job.payload.clone())
            .map_err(|error| Error::internal(format!("bulk job payload: {error}")))?;
        let (_, action) =
            self.actions.find(&payload.resource, &payload.action).ok_or_else(|| {
                Error::internal(format!(
                    "no bulk action `{}` on `{}`",
                    payload.action, payload.resource
                ))
            })?;
        let principal = payload.principal.restore();
        let total = u64::try_from(payload.ids.len()).unwrap_or(u64::MAX);
        let mut items = Vec::with_capacity(payload.ids.len());
        let (mut ok, mut skipped, mut failed) = (0u64, 0u64, 0u64);
        for (done, id) in payload.ids.iter().enumerate() {
            let item = match action.apply(&tenant, &principal, id).await {
                Ok(BulkOutcome::Done) => {
                    ok += 1;
                    BulkItem { id: id.clone(), status: "ok".to_owned(), code: None }
                }
                Ok(BulkOutcome::Skipped(reason)) => {
                    skipped += 1;
                    BulkItem {
                        id: id.clone(),
                        status: "skipped".to_owned(),
                        code: Some(reason.to_owned()),
                    }
                }
                Err(error) => {
                    failed += 1;
                    BulkItem {
                        id: id.clone(),
                        status: "failed".to_owned(),
                        code: Some(error.code().to_owned()),
                    }
                }
            };
            items.push(item);
            if done % 20 == 19 {
                let done = u64::try_from(done + 1).unwrap_or(u64::MAX);
                runs::progress(tenant.database().write(), job.id, done, total).await?;
            }
        }
        runs::progress(tenant.database().write(), job.id, total, total).await?;
        runs::set_result(
            tenant.database().write(),
            job.id,
            &serde_json::json!({ "ok": ok, "skipped": skipped, "failed": failed, "items": items }),
        )
        .await
    }
}
