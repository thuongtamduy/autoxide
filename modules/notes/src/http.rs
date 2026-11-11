//! Endpoint REST của module `notes`, dưới `/api/v1/notes`. Handler chỉ: lấy dữ liệu vào
//! → kiểm permission → gọi service → trả envelope (`Data`, `Page`; lỗi là `Problem`).
//! Mọi route đăng ký qua `routes!` để có tài liệu OpenAPI (luật kiến trúc 6).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{NOTES_READ, NOTES_WRITE};
use crate::service::{NewNote, Note, NoteChanges, NoteService};

pub(crate) fn routes(service: Arc<NoteService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_notes, create_note))
        .routes(routes!(get_note, update_note, delete_note))
        .with_state(service)
}

/// A note.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct NoteResponse {
    id: Uuid,
    /// Unique among the notes of one user.
    #[schema(example = "Shopping list")]
    title: String,
    #[schema(example = "Milk, eggs, bread.")]
    body: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<Note> for NoteResponse {
    fn from(note: Note) -> Self {
        NoteResponse {
            id: note.id,
            title: note.title,
            body: note.body,
            created_at: note.created_at,
            updated_at: note.updated_at,
        }
    }
}

/// A new note.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateNoteRequest {
    /// Title, 1 to 200 characters; unique among your notes.
    #[validate(length(min = 1, max = 200))]
    #[schema(example = "Shopping list")]
    title: String,
    /// Text of the note, up to 10000 characters.
    #[validate(length(max = 10000))]
    #[serde(default)]
    body: String,
}

impl From<CreateNoteRequest> for NewNote {
    fn from(input: CreateNoteRequest) -> Self {
        NewNote { title: input.title, body: input.body }
    }
}

/// Changes to a note. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateNoteRequest {
    /// New title, 1 to 200 characters.
    #[validate(length(min = 1, max = 200))]
    title: Option<String>,
    /// New text, up to 10000 characters.
    #[validate(length(max = 10000))]
    body: Option<String>,
}

/// List your notes.
///
/// Requires `notes.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/",
    operation_id = "list_notes",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your notes.", body = Page<NoteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_notes(
    State(service): State<Arc<NoteService>>,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<NoteResponse>, Error> {
    principal.require(&NOTES_READ)?;
    let page = service.list(Actor::from(&principal), &PageRequest::try_from(query)?).await?;
    Ok(page.map(NoteResponse::from))
}

/// Create a note.
///
/// Requires `notes.write`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/",
    operation_id = "create_note",
    security(("bearer" = [])),
    request_body = CreateNoteRequest,
    responses(
        (status = 201, description = "Created.", body = Data<NoteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.write`; code `permission_denied`.", body = Problem),
        (status = 409, description = "You already have a note with this title; code `notes.title_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_note(
    State(service): State<Arc<NoteService>>,
    principal: Principal,
    ValidJson(input): ValidJson<CreateNoteRequest>,
) -> Result<(StatusCode, Data<NoteResponse>), Error> {
    principal.require(&NOTES_WRITE)?;
    let note = service.create(Actor::from(&principal), input.into()).await?;
    Ok((StatusCode::CREATED, Data { data: note.into() }))
}

/// Get one of your notes.
///
/// Requires `notes.read`.
#[utoipa::path(
    get,
    path = "/{id}",
    operation_id = "get_note",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the note.")),
    responses(
        (status = 200, description = "The note.", body = Data<NoteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "You have no note with this id; code `notes.note_not_found`.", body = Problem),
    )
)]
async fn get_note(
    State(service): State<Arc<NoteService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<NoteResponse>, Error> {
    principal.require(&NOTES_READ)?;
    Ok(Data { data: service.get(Actor::from(&principal), id).await?.into() })
}

/// Change the title or text of one of your notes.
///
/// Requires `notes.write`.
#[utoipa::path(
    patch,
    path = "/{id}",
    operation_id = "update_note",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the note.")),
    request_body = UpdateNoteRequest,
    responses(
        (status = 200, description = "The note after the change.", body = Data<NoteResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "You have no note with this id; code `notes.note_not_found`.", body = Problem),
        (status = 409, description = "You already have a note with this title; code `notes.title_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn update_note(
    State(service): State<Arc<NoteService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<UpdateNoteRequest>,
) -> Result<Data<NoteResponse>, Error> {
    principal.require(&NOTES_WRITE)?;
    let changes = NoteChanges { title: input.title, body: input.body };
    Ok(Data { data: service.update(Actor::from(&principal), id, changes).await?.into() })
}

/// Delete one of your notes.
///
/// Requires `notes.write`.
#[utoipa::path(
    delete,
    path = "/{id}",
    operation_id = "delete_note",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the note.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `notes.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "You have no note with this id (or it was already deleted); code `notes.note_not_found`.", body = Problem),
    )
)]
async fn delete_note(
    State(service): State<Arc<NoteService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&NOTES_WRITE)?;
    service.delete(Actor::from(&principal), id).await?;
    Ok(StatusCode::NO_CONTENT)
}
