//! Endpoint của nhãn: `/api/v1/media/tags`. Gắn nhãn cho item bằng `tags` khi tải lên hay
//! sửa item; ở đây chỉ liệt kê, đổi tên, xoá nhãn.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, SortChoicePageQuery};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::{page_request, sort};
use crate::library::tags::{TagSort, TagWithCount};
use crate::service::MediaService;

/// A tag. Items carry tags as names; tags are created when first given to an item.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct TagResponse {
    id: Uuid,
    /// Unique, ignoring case.
    #[schema(example = "brand")]
    name: String,
    /// Number of items (not in the trash) with this tag.
    #[schema(example = 12)]
    item_count: u64,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<TagWithCount> for TagResponse {
    fn from(found: TagWithCount) -> Self {
        TagResponse {
            id: found.tag.id,
            name: found.tag.name,
            item_count: found.items,
            created_at: found.tag.created_at,
            updated_at: found.tag.updated_at,
        }
    }
}

/// Which tags to list.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TagsQuery {
    /// Only tags whose name contains this text, ignoring case and accents.
    pub(crate) q: Option<String>,
    /// `name` (the default; A→Z unless `order` says otherwise) or `created_at` (newest first unless
    /// `order` says otherwise).
    #[param(value_type = Option<TagSort>, inline)]
    pub(crate) sort: Option<String>,
}

/// A new name for a tag.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RenameTagRequest {
    /// 1 to 50 characters, without `,`; unique, ignoring case.
    #[schema(example = "brand")]
    name: String,
}

/// List media tags.
///
/// Requires `media.read`. Each tag comes with the number of items carrying it. Sorted by
/// name (A to Z with `order=asc`) unless `sort=created_at`.
#[utoipa::path(
    get,
    path = "/tags",
    extensions(("x-permissions" = json!(["media.read"]))),
    operation_id = "list_media_tags",
    security(("bearer" = [])),
    params(TagsQuery, SortChoicePageQuery),
    responses(
        (status = 200, description = "One page of tags.", body = Page<TagResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_tags(
    State(service): State<Arc<MediaService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<TagsQuery>,
    Query(paging): Query<SortChoicePageQuery>,
) -> Result<Page<TagResponse>, Error> {
    // Kiểm quyền trước khi đọc tham số trang: người không có quyền nhận 403, không phải 422 (D-20261126-list-pages-cluster-2).
    principal.require(&crate::permissions::MEDIA_READ)?;
    let mut problems = Vec::new();
    let order: TagSort = sort(query.sort.as_deref(), "`name`, `created_at`", &mut problems);
    let page = page_request(paging, order.default_order(), problems)?;
    Ok(service
        .list_tags(&tenant, &principal, query.q.as_deref(), order, &page)
        .await?
        .map(TagResponse::from))
}

/// Get a media tag.
///
/// Requires `media.read`.
#[utoipa::path(
    get,
    path = "/tags/{id}",
    extensions(("x-permissions" = json!(["media.read"]))),
    operation_id = "get_media_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    responses(
        (status = 200, description = "The tag.", body = Data<TagResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No tag has this id; code `media.tag_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_tag(
    State(service): State<Arc<MediaService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<TagResponse>, Error> {
    let found = service.get_tag(&tenant, &principal, id).await?;
    Ok(Tagged { version: found.tag.updated_at, data: found.into() })
}

/// Rename a media tag.
///
/// Requires `media.write`. Every item carrying the tag shows the new name. `If-Match` is
/// required.
#[utoipa::path(
    patch,
    path = "/tags/{id}",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "rename_media_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). The change is made only if nobody changed the tag since; otherwise `412`.")),
    request_body = RenameTagRequest,
    responses(
        (status = 200, description = "The tag after the change.", body = Data<TagResponse>, headers(("ETag" = String, description = "Version of the record: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No tag has this id; code `media.tag_not_found`.", body = Problem),
        (status = 409, description = "Another tag has this name; code `media.tag_name_taken`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the tag changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The name is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn rename_tag(
    State(service): State<Arc<MediaService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<RenameTagRequest>,
) -> Result<Tagged<TagResponse>, Error> {
    let expected = if_match.required()?;
    let found = service.rename_tag(&tenant, &principal, id, &input.name, expected).await?;
    Ok(Tagged { version: found.tag.updated_at, data: found.into() })
}

/// Delete a media tag.
///
/// Requires `media.write`. The tag is removed from every item; the items stay.
#[utoipa::path(
    delete,
    path = "/tags/{id}",
    extensions(("x-permissions" = json!(["media.write"]))),
    operation_id = "delete_media_tag",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the tag.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `media.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No tag has this id; code `media.tag_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_tag(
    State(service): State<Arc<MediaService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_tag(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
