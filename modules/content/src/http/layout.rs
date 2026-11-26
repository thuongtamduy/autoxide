//! Bố cục khối của trang (bước 67): thay cả (`PUT`) hay sửa từng phần bằng JSON Patch (`PATCH`),
//! luôn kèm `If-Match`. Kiểm bố cục, nháp và revision là của service.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use autoxide_spike_macros::Schema as ToSchema;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;
use validator::{Validate, ValidationErrors};

use super::entries::{EntryResponse, tagged};
use crate::entries::patch::Operation;
use crate::service::ContentService;

/// A whole block layout for a page.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LayoutRequest {
    /// `{"type": "layout", "schema": 1, "sections": [block…]}`. A block is
    /// `{"id", "block", "props"?, "visibility"?, "children"?}`: `id` is 1 to 64 letters, digits, `-`
    /// or `_`, unique in the page; `block` is a name from `GET /api/v1/kernel/blocks`; `props`
    /// follows that block's `props_schema`; `visibility` maps screen sizes `base` / `md` / `lg` to
    /// `true` / `false`; only blocks with `children: "blocks"` hold `children`. At most 500 blocks,
    /// 6 levels deep, 256 KB of JSON.
    #[schema(value_type = Object)]
    pub layout: Value,
    /// Keep this version in the history now, whatever the time since the last one.
    #[serde(default)]
    pub save_revision: bool,
}

/// A JSON Patch (RFC 6902): a list of operations applied in order, all or nothing.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(transparent)]
pub(crate) struct LayoutPatch(pub Vec<Operation>);

// `Validate` tự viết: kiểu bọc một danh sách, mỗi thao tác do service kiểm (lỗi kèm vị trí).
impl Validate for LayoutPatch {
    fn validate(&self) -> Result<(), ValidationErrors> {
        Ok(())
    }
}

/// Replace the block layout of a page.
///
/// Requires `content.write`, or `content.write_own` for a page you created. Changes the draft, like
/// `PATCH /entries/{id}` with a `layout` body: a published page keeps showing its published version
/// until it is published again. The whole layout is checked; every problem is listed at once with
/// its JSON path (e.g. `body.sections[0].props.text`). `If-Match` is required.
#[utoipa::path(
    put,
    path = "/entries/{id}/layout",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "put_content_entry_layout",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the page."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). The change is made only if nobody changed the page since; otherwise `412`.")),
    request_body = LayoutRequest,
    responses(
        (status = 200, description = "The page after the change.", body = Data<EntryResponse>, headers(("ETag" = String, description = "Version of the entry: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the page is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the page changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The layout is invalid, or the entry is a post (only pages have layouts); see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn put_layout(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<LayoutRequest>,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    let entry = service
        .replace_layout(&tenant, &principal, id, input.layout, input.save_revision, expected)
        .await?;
    tagged(&service, &tenant, entry).await
}

/// Change part of the block layout of a page with JSON Patch.
///
/// Requires `content.write`, or `content.write_own` for a page you created. The body is a JSON
/// Patch (RFC 6902, `application/json-patch+json` or `application/json`) on the layout document,
/// e.g. `[{"op": "replace", "path": "/sections/0/props/text", "value": "Sale"}]`; at most 200
/// operations, applied all or nothing, then the whole layout is checked as in `PUT`. A page whose
/// body is still an empty rich text document starts from an empty layout. `If-Match` is required.
#[utoipa::path(
    patch,
    path = "/entries/{id}/layout",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "patch_content_entry_layout",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the page."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). The change is made only if nobody changed the page since; otherwise `412`.")),
    request_body(content = LayoutPatch, content_type = "application/json-patch+json"),
    responses(
        (status = 200, description = "The page after the change.", body = Data<EntryResponse>, headers(("ETag" = String, description = "Version of the entry: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the page is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "The page has a rich text body, not a layout; replace it with `PUT` first; code `content.not_layout`.", body = Problem),
        (status = 412, description = "`If-Match` does not match: the page changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "An operation failed (its position in `errors`, e.g. `operations[2]`), or the resulting layout is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn patch_layout(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(patch): ValidJson<LayoutPatch>,
) -> Result<Tagged<EntryResponse>, Error> {
    let expected = if_match.required()?;
    let entry = service.patch_layout(&tenant, &principal, id, &patch.0, expected).await?;
    tagged(&service, &tenant, entry).await
}
