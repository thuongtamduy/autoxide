//! Endpoint REST của module `notes`, dưới `/api/v1/notes`. Handler chỉ: lấy dữ liệu vào
//! → kiểm permission → gọi service → trả envelope (`Data`, `Page`; lỗi là `Problem`).
//! Mọi route đăng ký qua `routes!` để có tài liệu OpenAPI (luật kiến trúc 6).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{Routes, Schema, ValidJson, endpoint};
use autoxide_kernel::routes;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::sync::{ChangesQuery, SyncChange};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{NOTES_READ, NOTES_WRITE};
use crate::service::{NewNote, Note, NoteChanges, NoteService};

pub(crate) fn routes(service: Arc<NoteService>) -> Routes {
    routes![list_notes, create_note, list_note_changes, get_note, update_note, delete_note].with_state(service)
}

/// A note.
#[derive(Debug, Serialize, Deserialize, Schema)]
pub(crate) struct NoteResponse {
    id: Uuid,
    /// The organization unit the note belongs to; the root unit has the id of the tenant.
    org_id: Uuid,
    /// Unique among the notes of one user.
    #[schema(example = "Shopping list")]
    title: String,
    #[schema(example = "Milk, eggs, bread.")]
    body: String,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<Note> for NoteResponse {
    fn from(note: Note) -> Self {
        NoteResponse {
            id: note.id,
            org_id: note.org_id.unwrap_or(note.tenant_id.as_uuid()),
            title: note.title,
            body: note.body,
            created_at: note.created_at,
            updated_at: note.updated_at,
        }
    }
}

/// A new note.
#[derive(Debug, Deserialize, Validate, Schema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateNoteRequest {
    /// Title, 1 to 200 characters; unique among your notes.
    #[schema(example = "Shopping list")]
    title: String,
    /// Text of the note, up to 10000 characters.
    #[serde(default)]
    body: String,
    /// The organization unit of the note; you need `notes.write` there. Omit when you work in
    /// one unit only (or for the whole tenant); otherwise `notes.org_required`.
    org_id: Option<Uuid>,
}

impl From<CreateNoteRequest> for NewNote {
    fn from(input: CreateNoteRequest) -> Self {
        NewNote { title: input.title, body: input.body, org_id: input.org_id }
    }
}

/// Changes to a note. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, Schema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateNoteRequest {
    /// New title, 1 to 200 characters.
    title: Option<String>,
    /// New text, up to 10000 characters.
    body: Option<String>,
}

/// List the notes in your scope.
///
/// Requires `notes.read`; its scope decides which notes you see (`own`: yours; `unit`, `subtree`:
/// notes of your unit, or of it and the units below; `tenant`: all). Newest first unless
/// `order=asc`.
#[endpoint(
    get,
    path = "/",
    permission = "notes.read",
    params(PageQuery),
    responses(
        (status = 200, description = "One page of the notes in your scope.", body = Page<NoteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_notes(
    State(service): State<Arc<NoteService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<NoteResponse>, Error> {
    principal.require(&NOTES_READ)?;
    let page = service.list(&tenant, &principal, &PageRequest::try_from(query)?).await?;
    Ok(page.map(NoteResponse::from))
}

/// List the notes changed since your last sync.
///
/// For offline apps (see `GET /api/v1/kernel/sync/resources`): each item is the note as it is
/// now, or `deleted` when it was deleted or left your scope. Keep the `cursor` of the last item you
/// applied and send it next time; an old cursor answers `410` (`sync_cursor_expired`): load the
/// list again. Requires `notes.read`; its scope decides which notes you get.
#[endpoint(
    get,
    path = "/changes",
    permission = "notes.read",
    params(ChangesQuery),
    responses(
        (status = 200, description = "The notes changed after `cursor`, oldest change first.", body = Page<SyncChange<NoteResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.read`; code `permission_denied`.", body = Problem),
        (status = 410, description = "`cursor` is older than the changes the server keeps; code `sync_cursor_expired`. Load the list again, then ask without `cursor`.", body = Problem),
        (status = 422, description = "`cursor` or `limit` is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_note_changes(
    State(service): State<Arc<NoteService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ChangesQuery>,
) -> Result<Page<SyncChange<NoteResponse>>, Error> {
    principal.require(&NOTES_READ)?;
    let page = service.changes(&tenant, &principal, &query).await?;
    Ok(page.map(|change| change.map(NoteResponse::from)))
}

/// Create a note.
///
/// Requires `notes.write` covering the unit of the note (`org_id`). Retry safely with an
/// `Idempotency-Key` header.
#[endpoint(
    post,
    path = "/",
    permission = "notes.write",
    request_body = CreateNoteRequest,
    responses(
        (status = 201, description = "Created.", body = Data<NoteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.write` (`permission_denied`), `org_id` is outside your scope (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 409, description = "You already have a note with this title; code `notes.title_taken`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or you work in several units and gave no `org_id` (`notes.org_required`).", body = Problem),
    )
)]
async fn create_note(
    State(service): State<Arc<NoteService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateNoteRequest>,
) -> Result<Created<NoteResponse>, Error> {
    principal.require(&NOTES_WRITE)?;
    let note = service.create(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/notes/{}", note.id), note.into()))
}

/// Get a note.
///
/// Requires `notes.read` covering the note; a note outside your scope answers 404.
#[endpoint(
    get,
    path = "/{id}",
    permission = "notes.read",
    params(("id" = Uuid, Path, description = "Id of the note.")),
    responses(
        (status = 200, description = "The note.", body = Data<NoteResponse>, etag),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No note with this id in your scope; code `notes.note_not_found`.", body = Problem),
    )
)]
async fn get_note(
    State(service): State<Arc<NoteService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<NoteResponse>, Error> {
    principal.require(&NOTES_READ)?;
    let note = service.get(&tenant, &principal, id).await?;
    Ok(Tagged { version: note.updated_at, data: note.into() })
}

/// Change the title or text of a note.
///
/// Requires `notes.write` covering the note; its unit must be active.
#[endpoint(
    patch,
    path = "/{id}",
    permission = "notes.write",
    params(("id" = Uuid, Path, description = "Id of the note."), ("If-Match" = Option<String>, Header, description = "The `ETag` you read. When given, the change is made only if nobody changed the record since; otherwise `412`.")),
    request_body = UpdateNoteRequest,
    responses(
        (status = 200, description = "The note after the change.", body = Data<NoteResponse>, etag),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.write` (`permission_denied`), it does not cover this note (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No note with this id in your scope; code `notes.note_not_found`.", body = Problem),
        (status = 409, description = "You already have a note with this title; code `notes.title_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the record changed since you read it; code `precondition_failed`. The body's `current` is the note as it is now and `ETag` its version: merge your change and retry with it.", body = Problem),
    )
)]
async fn update_note(
    State(service): State<Arc<NoteService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateNoteRequest>,
) -> Result<Tagged<NoteResponse>, Error> {
    principal.require(&NOTES_WRITE)?;
    let changes = NoteChanges { title: input.title, body: input.body };
    let expected = if_match.expected()?;
    let note = service.update(&tenant, &principal, id, changes, expected).await?;
    Ok(Tagged { version: note.updated_at, data: note.into() })
}

/// Delete a note.
///
/// Requires `notes.write` covering the note; its unit must be active.
#[endpoint(
    delete,
    path = "/{id}",
    permission = "notes.write",
    params(("id" = Uuid, Path, description = "Id of the note.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.write` (`permission_denied`), it does not cover this note (`org_out_of_scope`), or the unit is not active (`org_frozen`).", body = Problem),
        (status = 404, description = "No note with this id in your scope (or it was already deleted); code `notes.note_not_found`.", body = Problem),
    )
)]
async fn delete_note(
    State(service): State<Arc<NoteService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&NOTES_WRITE)?;
    service.delete(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
