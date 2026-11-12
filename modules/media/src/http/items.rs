//! Endpoint của item: danh sách, tải lên, xem, sửa thông tin, thùng rác.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Problem};
use autoxide_kernel::storage::ByteStream;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use futures_util::{StreamExt as _, TryStreamExt as _};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::{MediaItemResponse, folder_filter, page_of, page_request, present, sort, tagged};
use crate::config::normalize_type;
use crate::entity::api_folder;
use crate::service::{
    ItemChanges, ItemFilter, ItemSort, MediaService, Visibility, content_url, too_large,
};
use crate::upload::NewUpload;

/// Filters and order of the media list. Combine any of them.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ItemsQuery {
    /// Only items in this folder (not in its subfolders): a folder id, or `root`. Omit for
    /// every folder.
    #[param(example = "root")]
    pub folder_id: Option<String>,
    /// Only this media type (`image/png`) or family (`image/*`).
    #[serde(rename = "type")]
    #[param(rename = "type", example = "image/*")]
    pub content_type: Option<String>,
    /// Only `public` or only `private` items.
    #[param(value_type = Option<Visibility>, inline)]
    pub visibility: Option<String>,
    /// `true`: only items in the trash. Default `false`: only items not in the trash.
    #[param(value_type = Option<bool>)]
    pub trashed: Option<String>,
    /// Only items whose name contains this text, ignoring case and accents.
    #[param(example = "logo")]
    pub q: Option<String>,
    /// Only items with this tag, ignoring case.
    #[param(example = "brand")]
    pub tag: Option<String>,
    /// `created_at` (the default), `name` or `size`; the direction is `order`.
    #[param(value_type = Option<ItemSort>, inline)]
    pub sort: Option<String>,
}

impl ItemsQuery {
    /// Bộ lọc đã kiểm; mọi lỗi gom vào `problems`.
    pub(crate) fn filter(&self, problems: &mut Vec<FieldError>) -> ItemFilter {
        let content_type = self.content_type.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let content_type = content_type.and_then(|raw| {
            let wildcard = raw.strip_suffix("/*").map(|kind| format!("{kind}/x"));
            match normalize_type(wildcard.as_deref().unwrap_or(raw)) {
                Some(_) if wildcard.is_some() => Some(raw.to_ascii_lowercase()),
                Some(normalized) => Some(crate::sniff::canonical(&normalized).to_owned()),
                None => {
                    problems.push(FieldError::new(
                        "type",
                        "invalid",
                        "must be a media type like `image/png` or a family like `image/*`",
                    ));
                    None
                }
            }
        });
        let visibility = match self.visibility.as_deref().map(str::trim) {
            None | Some("") => None,
            Some("public") => Some(Visibility::Public),
            Some("private") => Some(Visibility::Private),
            Some(_) => {
                problems.push(FieldError::new(
                    "visibility",
                    "invalid",
                    "must be `public` or `private`",
                ));
                None
            }
        };
        let trashed = match self.trashed.as_deref().map(str::trim) {
            None | Some("") | Some("false") => false,
            Some("true") => true,
            Some(_) => {
                problems.push(FieldError::new("trashed", "invalid", "must be `true` or `false`"));
                false
            }
        };
        ItemFilter {
            folder: folder_filter(self.folder_id.as_deref(), "folder_id", problems),
            content_type,
            visibility,
            trashed,
            search: self.q.clone(),
            tag: self.tag.clone(),
            sort: sort(self.sort.as_deref(), "`created_at`, `name`, `size`", problems),
        }
    }
}

/// Details of an upload.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct UploadQuery {
    /// File name, 1 to 255 characters, without `/` or `\`.
    #[param(example = "team-photo.jpg")]
    name: String,
    /// Folder to put the item in. Omit for the root.
    folder_id: Option<Uuid>,
    /// Tags for the item, comma-separated (each 1 to 50 characters, at most 20).
    #[param(example = "brand,logo")]
    tags: Option<String>,
}

/// An item with the same content, found at upload.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct DuplicateItem {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    /// `null` for the root.
    pub(crate) folder_id: Option<Uuid>,
    pub(crate) url: String,
}

/// The uploaded item, plus the items already in the library with exactly the same content.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct UploadedResponse {
    #[serde(flatten)]
    pub(crate) item: MediaItemResponse,
    /// Items (not in the trash) whose content has the same SHA-256, oldest first, at most
    /// 20. Empty when the content is new. The upload is kept either way: delete it if you
    /// would rather reuse one of these.
    pub(crate) duplicates: Vec<DuplicateItem>,
}

/// Changes to a media item. Omitted fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateItemRequest {
    /// New file name, 1 to 255 characters, without `/` or `\`.
    pub name: Option<String>,
    /// Text alternative, up to 1000 characters; `""` clears it.
    pub alt: Option<String>,
    /// Title, up to 255 characters; `""` clears it.
    pub title: Option<String>,
    /// Caption, up to 2000 characters; `""` clears it.
    pub caption: Option<String>,
    /// Move to this folder; `null` moves to the root.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    pub folder_id: Option<Option<Uuid>>,
    /// `public` lets anyone load the content without signing in.
    pub visibility: Option<Visibility>,
    /// Replace the tags of the item (each 1 to 50 characters, without `,`; at most 20);
    /// `[]` removes them all. New tag names are created.
    #[schema(example = json!(["brand", "logo"]))]
    pub tags: Option<Vec<String>>,
}

impl From<UpdateItemRequest> for ItemChanges {
    fn from(input: UpdateItemRequest) -> Self {
        ItemChanges {
            name: input.name,
            alt: input.alt,
            title: input.title,
            caption: input.caption,
            folder_id: input.folder_id,
            visibility: input.visibility,
            tags: input.tags,
        }
    }
}

/// Luồng thân request; lỗi đọc (quá lớn, client ngắt) thành lỗi rõ ràng.
pub(super) fn body_stream(body: Body, max: u64) -> ByteStream {
    body.into_data_stream()
        .map_err(move |error| {
            if autoxide_kernel::http::is_too_large(&error) {
                too_large(max)
            } else {
                Error::bad_request(
                    "media.upload_interrupted",
                    "The upload was cut off before the whole content arrived. Send it again.",
                )
            }
        })
        .boxed()
}

/// Header `Content-Type` của request.
pub(super) fn content_type(headers: &HeaderMap) -> Result<Option<&str>, Error> {
    headers.get(header::CONTENT_TYPE).map(HeaderValue::to_str).transpose().map_err(|_| {
        Error::bad_request(
            "media.invalid_content_type",
            "The `Content-Type` header must be a media type like `image/png`.",
        )
    })
}

/// List media items.
///
/// Requires `media.read`. Filter by folder, type, visibility, trash and name; sort by
/// upload time, name or size. Keep the same filters, `sort` and `order` while following
/// `next_cursor`.
#[utoipa::path(
    get,
    path = "/",
    operation_id = "list_media_items",
    security(("bearer" = [])),
    params(ItemsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of media items.", body = Page<MediaItemResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_items(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Query(query): Query<ItemsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<MediaItemResponse>, Error> {
    let mut problems = Vec::new();
    let filter = query.filter(&mut problems);
    let page = page_request(paging, problems)?;
    page_of(&service, service.list(&principal, &filter, &page).await?).await
}

/// Upload a media item.
///
/// Requires `media.write`. Send the raw content as the body (not a multipart form), its
/// media type as `Content-Type`, and its name in `name`. The server checks the content
/// really is of that type (from its first bytes); without `Content-Type` (or with
/// `application/octet-stream`) it detects the type itself. Images get their width and
/// height. The new item is `private` until you change its `visibility`. Retry safely with
/// an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/",
    operation_id = "upload_media_item",
    security(("bearer" = [])),
    params(UploadQuery),
    request_body(
        content = Vec<u8>,
        content_type = "application/octet-stream",
        description = "The content of the file, with its `Content-Type`.",
    ),
    responses(
        (status = 201, description = "Uploaded. `duplicates` lists items that already had this content.", body = autoxide_kernel::response::Data<UploadedResponse>),
        (status = 400, description = "The `Content-Type` is not a media type (code `media.invalid_content_type`), or the upload was cut off (code `media.upload_interrupted`).", body = Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 413, description = "The content is too large; code `media.too_large` (or `payload_too_large` when `Content-Length` already says so).", body = Problem),
        (status = 415, description = "The type is not accepted (code `media.type_not_allowed`, the message lists the accepted types), the content is not of the declared type (code `media.content_mismatch`), the type could not be detected (code `media.unknown_type`), or the image is damaged (code `media.invalid_image`).", body = Problem),
        (status = 422, description = "The name is invalid, the folder does not exist, or the body is empty; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn upload_item(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<Created<UploadedResponse>, Error> {
    let input = NewUpload {
        name: query.name,
        folder_id: query.folder_id,
        content_type: content_type(&headers)?.map(str::to_owned),
        tags: query
            .tags
            .as_deref()
            .map(|tags| {
                tags.split(',').filter(|tag| !tag.trim().is_empty()).map(str::to_owned).collect()
            })
            .unwrap_or_default(),
    };
    let content = body_stream(body, service.config.limits.max_bytes);
    let uploaded = service.upload(&principal, input, content).await?;
    let duplicates = uploaded
        .duplicates
        .into_iter()
        .map(|item| DuplicateItem {
            url: content_url(&item),
            id: item.id,
            name: item.name,
            folder_id: api_folder(item.folder_id),
        })
        .collect();
    let item = super::describe(&service, uploaded.item).await?;
    let location = format!("/media/{}", item.id_ref());
    Ok(Created::new(location, UploadedResponse { item, duplicates }))
}

/// Get a media item.
///
/// Requires `media.read`. Items in the trash can be read too (see `trashed_at`).
#[utoipa::path(
    get,
    path = "/{id}",
    operation_id = "get_media_item",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item.")),
    responses(
        (status = 200, description = "The item.", body = autoxide_kernel::response::Data<MediaItemResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No media item has this id; code `media.item_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_item(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<MediaItemResponse>, Error> {
    let item = service.get(&principal, id).await?;
    tagged(&service, item).await
}

/// Change the details of a media item.
///
/// Requires `media.write`. Rename, describe (`alt`, `title`, `caption`), move to another
/// folder, or publish (`visibility`). `If-Match` is required. Restore an item from the
/// trash before changing it.
#[utoipa::path(
    patch,
    path = "/{id}",
    operation_id = "update_media_item",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). The change is made only if nobody changed the item since; otherwise `412`.")),
    request_body = UpdateItemRequest,
    responses(
        (status = 200, description = "The item after the change.", body = autoxide_kernel::response::Data<MediaItemResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No media item has this id; code `media.item_not_found`.", body = Problem),
        (status = 409, description = "The item is in the trash; code `media.item_in_trash`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the item changed since you read it. Get it again and retry; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid or the folder does not exist; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_item(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    autoxide_kernel::http::ValidJson(input): autoxide_kernel::http::ValidJson<UpdateItemRequest>,
) -> Result<Tagged<MediaItemResponse>, Error> {
    let expected = if_match.required()?;
    let item = service.update(&principal, id, input.into(), expected).await?;
    tagged(&service, item).await
}

/// Move a media item to the trash.
///
/// Requires `media.write`. Its public link stops working; restore it with
/// `POST /api/v1/media/{id}/restore`. Items stay in the trash for a number of days set by
/// the server (30 by default), then are deleted for good.
#[utoipa::path(
    delete,
    path = "/{id}",
    operation_id = "trash_media_item",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item.")),
    responses(
        (status = 200, description = "The item, now in the trash.", body = autoxide_kernel::response::Data<MediaItemResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No media item has this id; code `media.item_not_found`.", body = Problem),
        (status = 409, description = "The item is already in the trash; code `media.item_in_trash`.", body = Problem),
    )
)]
pub(crate) async fn trash_item(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<MediaItemResponse>, Error> {
    let item = service.trash(&principal, id).await?;
    tagged(&service, item).await
}

/// Restore a media item from the trash.
///
/// Requires `media.write`.
#[utoipa::path(
    post,
    path = "/{id}/restore",
    operation_id = "restore_media_item",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item.")),
    responses(
        (status = 200, description = "The item, out of the trash.", body = autoxide_kernel::response::Data<MediaItemResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No media item has this id; code `media.item_not_found`.", body = Problem),
        (status = 409, description = "The item is not in the trash; code `media.item_not_in_trash`.", body = Problem),
    )
)]
pub(crate) async fn restore_item(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<MediaItemResponse>, Error> {
    let item = service.restore(&principal, id).await?;
    tagged(&service, item).await
}

/// Delete a media item in the trash for good.
///
/// Requires `media.purge`. The item must be in the trash first. Its content is deleted
/// too; this cannot be undone.
#[utoipa::path(
    post,
    path = "/{id}/purge",
    operation_id = "purge_media_item",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the media item.")),
    responses(
        (status = 204, description = "Deleted for good."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.purge`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No media item has this id (or it was already deleted); code `media.item_not_found`.", body = Problem),
        (status = 409, description = "The item is not in the trash; code `media.item_not_in_trash`.", body = Problem),
    )
)]
pub(crate) async fn purge_item(
    State(service): State<Arc<MediaService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.purge(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

impl MediaItemResponse {
    fn id_ref(&self) -> Uuid {
        self.id
    }
}
