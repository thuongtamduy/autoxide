//! Endpoint của lịch sử phiên bản và lịch đăng / gỡ.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use autoxide_spike_macros::Schema as ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::entries::{EntryResponse, tagged};
use super::page_request;
use crate::entries::revisions::Revision;
use crate::publishing::schedules::{Action, Schedule};
use crate::service::ContentService;

/// A revision: a full copy of the editable fields of an entry at one moment.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct RevisionResponse {
    id: Uuid,
    entry_id: Uuid,
    /// Why it was kept: `initial`, `explicit_save`, `background_save`, `published`,
    /// `unpublished`, `restored` or `scheduled`.
    reason: String,
    created_by: Option<Uuid>,
    /// Display name of `created_by`; `null` when nobody is recorded, the account was deleted, or
    /// the server runs without user accounts.
    created_by_name: Option<String>,
    created_at: Timestamp,
    /// The fields (`title`, `slug`, `excerpt`, `body`, media, SEO). Only when reading one
    /// revision.
    #[schema(value_type = Option<Object>)]
    snapshot: Option<Value>,
}

pub(crate) fn revision(row: Revision, with_snapshot: bool) -> RevisionResponse {
    RevisionResponse {
        snapshot: with_snapshot.then(|| serde_json::from_str(&row.snapshot).unwrap_or(Value::Null)),
        id: row.id,
        entry_id: row.entry_id,
        reason: row.reason,
        created_by: row.created_by,
        created_by_name: None,
        created_at: row.created_at,
    }
}

/// Điền `created_by_name` cho các phiên bản bằng một lần tra theo lô.
async fn with_names(
    service: &ContentService,
    tenant: &TenantContext,
    mut revisions: Vec<RevisionResponse>,
) -> Result<Vec<RevisionResponse>, Error> {
    let ids: Vec<Uuid> = revisions.iter().filter_map(|r| r.created_by).collect();
    let names = service.user_names(tenant, &ids).await?;
    for revision in &mut revisions {
        revision.created_by_name = revision.created_by.and_then(|id| names.get(&id).cloned());
    }
    Ok(revisions)
}

/// A scheduled publish or take-down.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ScheduleResponse {
    id: Uuid,
    entry_id: Uuid,
    action: Action,
    /// The revision that will be published (for `publish`).
    revision_id: Option<Uuid>,
    run_at: Timestamp,
    /// `scheduled`, `succeeded`, `failed` or `canceled`.
    status: String,
    /// Why it failed, an error code such as `content.schedule_outdated`.
    error_code: Option<String>,
    created_by: Option<Uuid>,
    /// `true` when whoever scheduled it confirmed that the AI-generated content was reviewed.
    ai_reviewed: bool,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<Schedule> for ScheduleResponse {
    fn from(row: Schedule) -> Self {
        ScheduleResponse {
            action: if row.action == "unpublish" { Action::Unpublish } else { Action::Publish },
            id: row.id,
            entry_id: row.entry_id,
            revision_id: row.revision_id,
            run_at: row.run_at,
            status: row.status,
            error_code: row.error_code,
            created_by: row.created_by,
            ai_reviewed: row.ai_reviewed_by.is_some(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A new schedule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScheduleRequest {
    action: Action,
    /// When to run; must be in the future.
    run_at: Timestamp,
    /// For a `publish` of AI-generated content: you confirm a person reviewed it, as with
    /// `ai_reviewed` when publishing at once. Only a signed-in person may send `true`.
    #[serde(default)]
    ai_reviewed: bool,
}

/// List the revisions of an entry.
///
/// Requires `content.read`. Newest first unless `order=asc`; without the snapshots (get one
/// revision to see its fields).
#[utoipa::path(
    get,
    path = "/entries/{id}/revisions",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_revisions",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), PageQuery),
    responses(
        (status = 200, description = "One page of revisions.", body = Page<RevisionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
    )
)]
pub(crate) async fn list_revisions(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<RevisionResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let mut page = service
        .list_revisions(&tenant, &principal, id, &page)
        .await?
        .map(|row| revision(row, false));
    page.data = with_names(&service, &tenant, page.data).await?;
    Ok(page)
}

/// Get one revision of an entry, with its fields.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/entries/{id}/revisions/{revision_id}",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_revision",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("revision_id" = Uuid, Path, description = "Id of the revision.")),
    responses(
        (status = 200, description = "The revision.", body = Data<RevisionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such revision of this entry; code `content.revision_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_revision(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, revision_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<RevisionResponse>, Error> {
    let mut data =
        revision(service.get_revision(&tenant, &principal, id, revision_id).await?, true);
    if let Some(by) = data.created_by {
        data.created_by_name = service.user_names(&tenant, &[by]).await?.remove(&by);
    }
    Ok(Data { data })
}

/// Copy a revision back into the draft.
///
/// Requires `content.read`, and `content.write` (or `content.write_own` for an entry you
/// created). Does not publish. Categories, tags, authors and the parent page
/// are not part of revisions and stay as they are. `If-Match` is required.
#[utoipa::path(
    post,
    path = "/entries/{id}/revisions/{revision_id}/restore",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "restore_content_revision",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("revision_id" = Uuid, Path, description = "Id of the revision."), ("If-Match" = String, Header, description = "Required: the `ETag` of the entry (or `*`).")),
    responses(
        (status = 200, description = "The draft, now with the revision's fields.", body = Data<EntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No such entry or revision; code `content.not_found` or `content.revision_not_found`.", body = Problem),
        (status = 409, description = "The revision's slug is now taken; code `content.slug_taken`.", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A media item of the revision no longer exists; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn restore_revision(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, revision_id)): Path<(Uuid, Uuid)>,
    if_match: IfMatch,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    tagged(
        &service,
        &tenant,
        service.restore(&tenant, &principal, id, revision_id, expected).await?,
    )
    .await
}

/// Which schedules to list.
#[derive(Debug, Clone, Default, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SchedulesQuery {
    /// `scheduled` (waiting), `succeeded`, `failed` or `canceled`.
    #[param(example = "scheduled")]
    status: Option<String>,
}

/// List the schedules of every entry.
///
/// Requires `content.read`. Sorted by `run_at`, latest first unless `order=asc` (soonest
/// first — what is about to be published or taken down). Filter with `status=scheduled` for the
/// ones still waiting.
#[utoipa::path(
    get,
    path = "/schedules",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_all_content_schedules",
    security(("bearer" = [])),
    params(SchedulesQuery, PageQuery),
    responses(
        (status = 200, description = "One page of schedules.", body = Page<ScheduleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_all_schedules(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<SchedulesQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ScheduleResponse>, Error> {
    let mut problems = Vec::new();
    let status = query.status.as_deref().map(str::trim).filter(|status| !status.is_empty());
    if status
        .is_some_and(|status| !["scheduled", "succeeded", "failed", "canceled"].contains(&status))
    {
        problems.push(autoxide_kernel::error::FieldError::new(
            "status",
            "invalid",
            "must be `scheduled`, `succeeded`, `failed` or `canceled`",
        ));
    }
    let page = page_request(paging, problems)?;
    let page = service.list_all_schedules(&tenant, &principal, status, &page).await?;
    Ok(page.map(ScheduleResponse::from))
}

/// List the schedules of an entry.
///
/// Requires `content.read`. Newest first unless `order=asc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/entries/{id}/schedules",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_schedules",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), PageQuery),
    responses(
        (status = 200, description = "One page of the schedules, newest first.", body = Page<ScheduleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_schedules(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ScheduleResponse>, Error> {
    // Kiểm quyền trước khi đọc tham số trang: người không có quyền nhận 403, không phải 422 (D-20261126-list-pages-cluster-2).
    principal.require(&crate::permissions::CONTENT_READ)?;
    let page = PageRequest::try_from(paging)?;
    let rows = service.list_schedules(&tenant, &principal, id, &page).await?;
    Ok(rows.map(ScheduleResponse::from))
}

/// Schedule a publish or take-down.
///
/// Requires `content.publish`. A `publish` schedule publishes the draft as it is now, even if
/// it is edited later; it fails (`content.schedule_outdated`) when the entry was published by
/// other means after the schedule was made. At most one pending schedule of each action per
/// entry. `If-Match` is required.
#[utoipa::path(
    post,
    path = "/entries/{id}/schedules",
    extensions(("x-permissions" = json!(["content.publish"]))),
    operation_id = "create_content_schedule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry."), ("If-Match" = String, Header, description = "Required: the `ETag` of the entry (or `*`).")),
    request_body = ScheduleRequest,
    responses(
        (status = 201, description = "Scheduled.", body = Data<ScheduleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.publish`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "A schedule of this action is already pending; code `content.schedule_conflict`.", body = Problem),
        (status = 412, description = "The entry changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "`run_at` is not in the future (`schedule_in_past`), or a media item is missing or not public; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn create_schedule(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ScheduleRequest>,
) -> Result<Created<ScheduleResponse>, Error> {
    let expected = if_match.required()?;
    let row = service
        .create_schedule_reviewed(
            &tenant,
            &principal,
            id,
            crate::publishing::schedules::SchedulePlan {
                action: input.action,
                run_at: input.run_at,
                ai_reviewed: input.ai_reviewed,
            },
            expected,
        )
        .await?;
    Ok(Created::new(format!("/content/entries/{id}/schedules"), row.into()))
}

/// Cancel a pending schedule.
///
/// Requires `content.publish`.
#[utoipa::path(
    delete,
    path = "/schedules/{id}",
    extensions(("x-permissions" = json!(["content.publish"]))),
    operation_id = "cancel_content_schedule",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the schedule.")),
    responses(
        (status = 200, description = "Canceled.", body = Data<ScheduleResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.publish`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No schedule has this id; code `content.schedule_not_found`.", body = Problem),
        (status = 409, description = "It already ran or was canceled; code `content.schedule_not_pending`.", body = Problem),
    )
)]
pub(crate) async fn cancel_schedule(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ScheduleResponse>, Error> {
    Ok(Data { data: service.cancel_schedule(&tenant, &principal, id).await?.into() })
}
