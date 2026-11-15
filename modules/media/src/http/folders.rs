//! Endpoint của thư mục: `/api/v1/media/folders`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::{folder_filter, page_request, present, sort};
use crate::folders::{FolderChanges, FolderSort};
use crate::service::MediaService;

/// A media folder. Folders nest; names are unique (ignoring case) within a parent.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FolderResponse {
    pub(super) id: Uuid,
    /// The parent folder; `null` for a folder at the root.
    pub(super) parent_id: Option<Uuid>,
    #[schema(example = "Products")]
    pub(super) name: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) updated_at: DateTime<Utc>,
}

/// Which folders to list.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FoldersQuery {
    /// Only the folders directly inside this one: a folder id, or `root`. Omit for every
    /// folder.
    #[param(example = "root")]
    parent_id: Option<String>,
    /// `created_at` (the default) or `name`; the direction is `order`.
    #[param(value_type = Option<FolderSort>, inline)]
    sort: Option<String>,
}

/// A new folder.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFolderRequest {
    /// Name, 1 to 255 characters, without `/` or `\`; unique (ignoring case) in the parent.
    #[schema(example = "Products")]
    name: String,
    /// The parent folder. Omit (or `null`) for the root.
    #[serde(default)]
    parent_id: Option<Uuid>,
}

/// Changes to a folder. Omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateFolderRequest {
    /// New name.
    name: Option<String>,
    /// Move into this folder; `null` moves to the root. Not into itself or its subfolders.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    parent_id: Option<Option<Uuid>>,
}

/// List media folders.
///
/// Requires `media.read`.
#[utoipa::path(
    get,
    path = "/folders",
    extensions(("x-permissions" = json!(["media.read"]))),
    operation_id = "list_media_folders",
    security(("bearer" = [])),
    params(FoldersQuery, PageQuery),
    responses(
        (status = 200, description = "One page of folders.", body = Page<FolderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_folders(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Query(query): Query<FoldersQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<FolderResponse>, Error> {
    let mut problems = Vec::new();
    let parent = folder_filter(query.parent_id.as_deref(), "parent_id", &mut problems);
    let order: FolderSort = sort(query.sort.as_deref(), "`created_at`, `name`", &mut problems);
    let page = page_request(paging, problems)?;
    Ok(service.list_folders(&principal, parent, order, &page).await?.map(FolderResponse::from))
}

/// Create a media folder.
///
/// Requires `media.write`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/folders",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "create_media_folder",
    security(("bearer" = [])),
    request_body = CreateFolderRequest,
    responses(
        (status = 201, description = "Created.", body = Data<FolderResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The parent already holds a folder with this name; code `media.folder_name_taken`.", body = Problem),
        (status = 422, description = "The name is invalid or the parent does not exist; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_folder(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    ValidJson(input): ValidJson<CreateFolderRequest>,
) -> Result<Created<FolderResponse>, Error> {
    let folder = service.create_folder(&principal, &input.name, input.parent_id).await?;
    Ok(Created::new(format!("/media/folders/{}", folder.id), folder.into()))
}

/// Get a media folder.
///
/// Requires `media.read`.
#[utoipa::path(
    get,
    path = "/folders/{id}",
    extensions(("x-permissions" = json!(["media.read"]))),
    operation_id = "get_media_folder",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the folder.")),
    responses(
        (status = 200, description = "The folder.", body = Data<FolderResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No folder has this id; code `media.folder_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_folder(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<FolderResponse>, Error> {
    let folder = service.get_folder(&principal, id).await?;
    Ok(Tagged { version: folder.updated_at, data: folder.into() })
}

/// Rename or move a media folder.
///
/// Requires `media.write`. `If-Match` is required.
#[utoipa::path(
    patch,
    path = "/folders/{id}",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "update_media_folder",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the folder."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). The change is made only if nobody changed the folder since; otherwise `412`.")),
    request_body = UpdateFolderRequest,
    responses(
        (status = 200, description = "The folder after the change.", body = Data<FolderResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No folder has this id; code `media.folder_not_found`.", body = Problem),
        (status = 409, description = "The new parent already holds a folder with this name (code `media.folder_name_taken`), or is the folder itself or one of its subfolders (code `media.folder_cycle`).", body = Problem),
        (status = 412, description = "`If-Match` does not match: the folder changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The name is invalid or the parent does not exist; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_folder(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateFolderRequest>,
) -> Result<Tagged<FolderResponse>, Error> {
    let expected = if_match.required()?;
    let changes = FolderChanges { name: input.name, parent_id: input.parent_id };
    let folder = service.update_folder(&principal, id, changes, expected).await?;
    Ok(Tagged { version: folder.updated_at, data: folder.into() })
}

/// Delete an empty media folder.
///
/// Requires `media.write`. The folder must hold no folders and no items, not even items
/// in the trash.
#[utoipa::path(
    delete,
    path = "/folders/{id}",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "delete_media_folder",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the folder.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No folder has this id; code `media.folder_not_found`.", body = Problem),
        (status = 409, description = "The folder is not empty; code `media.folder_not_empty`.", body = Problem),
    )
)]
pub(crate) async fn delete_folder(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_folder(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
