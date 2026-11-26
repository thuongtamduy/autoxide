//! Trang mẫu (bước 67): danh sách, cài bộ mẫu dựng sẵn, lưu trang thành mẫu, xoá, tạo trang từ mẫu.
//! Logic ở `page_templates.rs`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use autoxide_spike_macros::Schema as ToSchema;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::IntoParams;
use uuid::Uuid;
use validator::Validate;

use super::entries::{EntryResponse, tagged};
use super::page_request;
use crate::page_templates::{FromTemplate, PageTemplate, SaveTemplate};
use crate::service::ContentService;

/// A page template: a block layout to start a page from.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct PageTemplateResponse {
    id: Uuid,
    org_id: Uuid,
    /// `retail`, `fnb`, `services`, `events`, `recruiting` or `other`.
    industry: String,
    /// Language of the template's text.
    locale: String,
    name: String,
    description: String,
    /// The block layout copied into a new page.
    #[schema(value_type = Object)]
    layout: Value,
    /// Key in the templates shipped with the server, when installed from them; `null`: saved by
    /// someone of the tenant.
    preset_key: Option<String>,
    created_by: Option<Uuid>,
    #[schema(value_type = String, format = DateTime)]
    created_at: Timestamp,
}

fn response(row: PageTemplate) -> PageTemplateResponse {
    PageTemplateResponse {
        layout: serde_json::from_str(&row.layout).unwrap_or(Value::Null),
        id: row.id,
        org_id: row.org_id,
        industry: row.industry,
        locale: row.locale,
        name: row.name,
        description: row.description,
        preset_key: row.preset_key,
        created_by: row.created_by,
        created_at: row.created_at,
    }
}

/// Which templates to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TemplatesQuery {
    /// Only this industry: `retail`, `fnb`, `services`, `events`, `recruiting`, `other`.
    industry: Option<String>,
    /// Only templates in this language.
    locale: Option<String>,
}

/// List page templates.
///
/// Requires `content.read`. Install the templates shipped with the server once with
/// `POST /page-templates/presets`.
#[utoipa::path(
    get,
    path = "/page-templates",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_page_templates",
    security(("bearer" = [])),
    params(TemplatesQuery, PageQuery),
    responses(
        (status = 200, description = "A page of templates.", body = Page<PageTemplateResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_page_templates(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<TemplatesQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<PageTemplateResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let found = service
        .list_page_templates(
            &tenant,
            &principal,
            query.industry.as_deref(),
            query.locale.as_deref(),
            &page,
        )
        .await?;
    Ok(found.map(response))
}

/// A page to keep as a template.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveTemplateRequest {
    /// A page with a block layout; its current draft is copied.
    entry_id: Uuid,
    /// 1 to 120 characters.
    name: String,
    /// `retail`, `fnb`, `services`, `events`, `recruiting` or `other`.
    industry: String,
    /// At most 500 characters.
    description: Option<String>,
}

/// Save a page as a template.
///
/// Requires `content.read` on the page and `content.write` at its unit. The template keeps the
/// page's language and unit.
#[utoipa::path(
    post,
    path = "/page-templates",
    extensions(("x-permissions" = json!(["content.write"]))),
    operation_id = "create_content_page_template",
    security(("bearer" = [])),
    request_body = SaveTemplateRequest,
    responses(
        (status = 201, description = "Created.", body = Data<PageTemplateResponse>, headers(("Location" = String, description = "Address of the templates list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` at the page's unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No page has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "The name, industry or description is invalid, or the page has a rich text body; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn save_page_template(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SaveTemplateRequest>,
) -> Result<Created<PageTemplateResponse>, Error> {
    let input = SaveTemplate {
        entry_id: input.entry_id,
        name: input.name,
        industry: input.industry,
        description: input.description,
    };
    let row = service.save_page_template(&tenant, &principal, input).await?;
    let location = format!("/api/v1/content/page-templates?industry={}", row.industry);
    Ok(Created::new(location, response(row)))
}

/// Install the page templates shipped with the server.
///
/// Requires `content.write` for the whole tenant. Adds the templates (retail, food and drink,
/// services, events, recruiting) missing in each language the tenant writes in; running it again
/// adds nothing twice. Returns only what was added.
#[utoipa::path(
    post,
    path = "/page-templates/presets",
    extensions(("x-permissions" = json!(["content.write"]))),
    operation_id = "install_content_page_templates",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The templates added (empty when all were there).", body = Data<Vec<PageTemplateResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` for the whole tenant; code `permission_denied` or `org_out_of_scope`.", body = Problem),
    )
)]
pub(crate) async fn install_page_templates(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Vec<PageTemplateResponse>>, Error> {
    let added = service.install_page_templates(&tenant, &principal).await?;
    Ok(Data { data: added.into_iter().map(response).collect() })
}

/// Delete a page template.
///
/// Requires `content.write` at the template's unit. Pages made from it are not changed.
#[utoipa::path(
    delete,
    path = "/page-templates/{id}",
    extensions(("x-permissions" = json!(["content.write"]))),
    operation_id = "delete_content_page_template",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the template.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No template has this id; code `content.page_template_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_page_template(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_page_template(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A new page from a template.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FromTemplateRequest {
    template_id: Uuid,
    /// Title of the new page.
    title: String,
    /// Parent page, in the template's language.
    parent_id: Option<Uuid>,
    /// Owning organization unit; left out: your only unit, or the whole tenant.
    org_id: Option<Uuid>,
}

/// The draft made from a template.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct FromTemplateResponse {
    entry: EntryResponse,
    /// Blocks of the template left out because the tenant cannot use them (module off, or a plan
    /// without the feature), by name.
    dropped_blocks: Vec<String>,
}

/// Create a page from a template.
///
/// Requires `content.read` and `content.write` (or `content.write_own`). The page is a draft in the
/// template's language with a copy of its layout; edit and publish it as any page.
#[utoipa::path(
    post,
    path = "/entries/from-template",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "create_content_entry_from_template",
    security(("bearer" = [])),
    request_body = FromTemplateRequest,
    responses(
        (status = 201, description = "The draft page.", body = Data<FromTemplateResponse>, headers(("Location" = String, description = "Address of the new page."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission; code `permission_denied`.", body = Problem),
        (status = 404, description = "No template has this id; code `content.page_template_not_found`.", body = Problem),
        (status = 409, description = "The slug from the title is taken; code `content.slug_taken`.", body = Problem),
        (status = 422, description = "The title, parent or a block of the template is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn page_from_template(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<FromTemplateRequest>,
) -> Result<Created<FromTemplateResponse>, Error> {
    let input = FromTemplate {
        template_id: input.template_id,
        title: input.title,
        parent_id: input.parent_id,
        org_id: input.org_id,
    };
    let (entry, dropped_blocks) = service.page_from_template(&tenant, &principal, input).await?;
    let location = format!("/api/v1/content/entries/{}", entry.id);
    let entry = tagged(&service, &tenant, entry).await?.data;
    Ok(Created::new(location, FromTemplateResponse { entry, dropped_blocks }))
}
