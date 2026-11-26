//! Khu dùng chung nhiều trang (bước 67): danh sách, đọc, tạo, thay, xoá. Logic ở `shared/`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use autoxide_spike_macros::Schema as ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::page_request;
use crate::service::ContentService;
use crate::shared::{SectionInput, SharedSection};

/// A shared section: blocks placed on many pages with the block `content.shared_section`.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct SharedSectionResponse {
    id: Uuid,
    /// The organization unit that owns it (the root unit: the whole tenant).
    org_id: Uuid,
    name: String,
    /// A block layout, as the `layout` body of a page.
    #[schema(value_type = Object)]
    layout: Value,
    created_by: Option<Uuid>,
    #[schema(value_type = String, format = DateTime)]
    created_at: Timestamp,
    #[schema(value_type = String, format = DateTime)]
    updated_at: Timestamp,
}

fn response(row: SharedSection) -> SharedSectionResponse {
    SharedSectionResponse {
        layout: serde_json::from_str(&row.layout).unwrap_or(Value::Null),
        id: row.id,
        org_id: row.org_id,
        name: row.name,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn tagged(row: SharedSection) -> Tagged<SharedSectionResponse> {
    Tagged { version: row.updated_at, data: response(row) }
}

/// A shared section to create or replace.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SharedSectionRequest {
    /// Name shown to editors, 1 to 120 characters.
    pub name: String,
    /// `{"type": "layout", "schema": 1, "sections": [block…]}`, checked like a page layout; it cannot
    /// hold another shared section.
    #[schema(value_type = Object)]
    pub layout: Value,
    /// Owning organization unit (create only); left out: the whole tenant.
    pub org_id: Option<Uuid>,
}

impl From<SharedSectionRequest> for SectionInput {
    fn from(input: SharedSectionRequest) -> Self {
        SectionInput { name: input.name, layout: input.layout, org_id: input.org_id }
    }
}

/// List shared sections.
///
/// Requires `content.read`; only sections of units in your reach. Paged by `id`.
#[utoipa::path(
    get,
    path = "/shared-sections",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_shared_sections",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of shared sections.", body = Page<SharedSectionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_shared_sections(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(paging): Query<PageQuery>,
) -> Result<Page<SharedSectionResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    Ok(service.list_shared_sections(&tenant, &principal, &page).await?.map(response))
}

/// Create a shared section.
///
/// Requires `content.write` at the owning unit. Place it on pages with the block
/// `content.shared_section {"id": …}`.
#[utoipa::path(
    post,
    path = "/shared-sections",
    extensions(("x-permissions" = json!(["content.write"]))),
    operation_id = "create_content_shared_section",
    security(("bearer" = [])),
    request_body = SharedSectionRequest,
    responses(
        (status = 201, description = "Created.", body = autoxide_kernel::response::Data<SharedSectionResponse>, headers(("Location" = String, description = "Address of the new section."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` at the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 422, description = "The name or the layout is invalid; see `errors` (layout paths start with `layout`).", body = Problem),
    )
)]
pub(crate) async fn create_shared_section(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SharedSectionRequest>,
) -> Result<Created<SharedSectionResponse>, Error> {
    let row = service.create_shared_section(&tenant, &principal, input.into()).await?;
    let location = format!("/api/v1/content/shared-sections/{}", row.id);
    Ok(Created::new(location, response(row)))
}

/// Get a shared section.
///
/// Requires `content.read` at its unit.
#[utoipa::path(
    get,
    path = "/shared-sections/{id}",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_shared_section",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shared section.")),
    responses(
        (status = 200, description = "The section.", body = autoxide_kernel::response::Data<SharedSectionResponse>, headers(("ETag" = String, description = "Version: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No section has this id; code `content.shared_section_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_shared_section(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<SharedSectionResponse>, Error> {
    Ok(tagged(service.shared_section(&tenant, &principal, id).await?))
}

/// Replace a shared section.
///
/// Requires `content.write` at its unit; `If-Match` is required. Every page using the section shows
/// the new blocks at once (no need to publish the pages again). `org_id` is ignored.
#[utoipa::path(
    put,
    path = "/shared-sections/{id}",
    extensions(("x-permissions" = json!(["content.write"]))),
    operation_id = "put_content_shared_section",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shared section."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). Otherwise `412`.")),
    request_body = SharedSectionRequest,
    responses(
        (status = 200, description = "The section after the change.", body = autoxide_kernel::response::Data<SharedSectionResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` at the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No section has this id; code `content.shared_section_not_found`.", body = Problem),
        (status = 412, description = "The section changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "The name or the layout is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn put_shared_section(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<SharedSectionRequest>,
) -> Result<Tagged<SharedSectionResponse>, Error> {
    let expected = if_match.required()?;
    Ok(tagged(service.put_shared_section(&tenant, &principal, id, input.into(), expected).await?))
}

/// Delete a shared section.
///
/// Requires `content.write` at its unit; `If-Match` is required. A section still used by a page
/// (its draft or its published version) is not deleted: `409` with the number of pages.
#[utoipa::path(
    delete,
    path = "/shared-sections/{id}",
    extensions(("x-permissions" = json!(["content.write"]))),
    operation_id = "delete_content_shared_section",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the shared section."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` at the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No section has this id; code `content.shared_section_not_found`.", body = Problem),
        (status = 409, description = "Pages still use it; code `content.shared_section_in_use` with `count`.", body = Problem),
        (status = 412, description = "The section changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn delete_shared_section(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<StatusCode, Error> {
    let expected = if_match.required()?;
    service.delete_shared_section(&tenant, &principal, id, expected).await?;
    Ok(StatusCode::NO_CONTENT)
}
