//! Endpoint tải thẳng lên S3 (bước 40 P4): bắt đầu lượt (link ký sẵn) và xác nhận.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::upload::UploadLink;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::items::{DuplicateItem, UploadedResponse};
use crate::entity::api_folder;
use crate::library::direct::{Confirmed, StartMediaUpload};
use crate::library::upload::Uploaded;
use crate::service::{MediaService, content_url};

/// A media item to upload straight to storage.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartMediaUploadRequest {
    /// File name, 1 to 255 characters, without `/` or `\`.
    #[schema(example = "launch-video.mp4")]
    pub(crate) name: String,
    /// The real media type of the file; the upload link only accepts this `Content-Type`, and the
    /// content is checked against it when you complete the upload.
    #[schema(example = "video/mp4")]
    pub(crate) content_type: String,
    /// Exact size in bytes.
    #[schema(example = 734003200)]
    pub(crate) size: u64,
    /// SHA-256 of the content, as 64 hex digits.
    pub(crate) sha256: String,
    /// Folder to put the item in. Omit for the root.
    pub(crate) folder_id: Option<Uuid>,
    /// Tags for the item (each 1 to 50 characters, at most 20).
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    /// The organization unit that owns the item; you need `media.write` there. Omit when you work
    /// in one unit only (or for the whole tenant).
    pub(crate) org_id: Option<Uuid>,
}

impl From<StartMediaUploadRequest> for StartMediaUpload {
    fn from(request: StartMediaUploadRequest) -> Self {
        StartMediaUpload {
            name: request.name,
            content_type: request.content_type,
            size: request.size,
            sha256: request.sha256,
            folder_id: request.folder_id,
            tags: request.tags,
            org_id: request.org_id,
        }
    }
}

/// Start uploading a media item straight to storage.
///
/// Requires `media.write`. For large items when files are stored on S3: the answer is a signed
/// link; send the file there (not to this server) with the given method and headers before
/// `expires_at`, then call `POST /api/v1/media/uploads/{id}/complete`. When files are stored on the
/// server's disk the answer is `409 media.direct_upload_unavailable`: upload through
/// `POST /api/v1/media`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/uploads",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "start_media_upload",
    security(("bearer" = [])),
    request_body = StartMediaUploadRequest,
    responses(
        (status = 201, description = "The upload link. `Location` is the item it will become once completed.", body = Data<UploadLink>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write` (code `permission_denied`), or not in the chosen unit (code `org_out_of_scope`).", body = Problem),
        (status = 409, description = "Files are stored where they cannot be uploaded straight to; code `media.direct_upload_unavailable`. Use `POST /api/v1/media`.", body = Problem),
        (status = 422, description = "The name, type, size, SHA-256, folder, tags or unit is invalid; see `errors` (all problems at once).", body = Problem),
    )
)]
pub(crate) async fn start_media_upload(
    State(service): State<Arc<MediaService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(request): ValidJson<StartMediaUploadRequest>,
) -> Result<Created<UploadLink>, Error> {
    let (upload, link) = service.start_upload(&tenant, &principal, request.into()).await?;
    Ok(Created::new(format!("/media/{}", upload.id), UploadLink::new(upload.id, link)))
}

/// Complete an upload of a media item straight to storage.
///
/// Requires `media.write` (your own uploads). The server reads the item back and checks it like
/// an upload through the server — its real type from its first bytes, no active SVG content, image
/// size — and its size and SHA-256 against what you declared. A file that fails is removed; send
/// the right file to the same link and complete again. Calling it again after it succeeded returns
/// the same item with `200`.
#[utoipa::path(
    post,
    path = "/uploads/{id}/complete",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "complete_media_upload",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the upload.")),
    responses(
        (status = 200, description = "Already completed: the same item.", body = Data<UploadedResponse>),
        (status = 201, description = "The item was created. `duplicates` lists items that already had this content.", body = Data<UploadedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No upload of yours has this id (code `media.upload_not_found`), or its item was deleted since (code `media.item_not_found`).", body = Problem),
        (status = 409, description = "The file is not in storage yet (code `media.upload_incomplete`), or the upload expired (code `media.upload_expired`).", body = Problem),
        (status = 413, description = "The file is larger than the server accepts; code `media.too_large`. It was removed.", body = Problem),
        (status = 415, description = "The content is not of the declared type (code `media.content_mismatch`), the SVG has active content (code `media.svg_active_content`) or the image is damaged (code `media.invalid_image`). It was removed.", body = Problem),
        (status = 422, description = "The file does not match the declared size or SHA-256 (code `media.upload_mismatch`; it was removed), or its folder was deleted since (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn complete_media_upload(
    State(service): State<Arc<MediaService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    let Confirmed { uploaded, created } = service.complete_upload(&tenant, &principal, id).await?;
    let response = present_uploaded(&service, &tenant, uploaded).await?;
    Ok(if created {
        Created::new(format!("/media/{id}"), response).into_response()
    } else {
        Data { data: response }.into_response()
    })
}

/// Item vừa tải lên, kèm item trùng nội dung — cùng hình dạng với `POST /api/v1/media`.
pub(crate) async fn present_uploaded(
    service: &MediaService,
    tenant: &TenantContext,
    uploaded: Uploaded,
) -> Result<UploadedResponse, Error> {
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
    let item = super::describe(service, tenant, uploaded.item).await?;
    Ok(UploadedResponse { item, duplicates })
}
