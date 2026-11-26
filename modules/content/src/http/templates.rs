//! Mẫu trang bắt buộc (bước 37, D-359): liệt kê và tạo trang nháp từ mẫu.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{NaturalOrderPageQuery, Page, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::entries::{EntryResponse, tagged};
use crate::service::ContentService;
use crate::templates::{TemplateDraft, TemplateSummary};

/// List the mandatory page templates (Vietnam).
///
/// Requires `content.read`. Starting texts in English and Vietnamese for the pages Vietnamese law
/// requires on a commerce website (owner information, personal data protection, returns, shipping,
/// payment, inspection, complaints, terms, general conditions; a marketplace adds its regulations,
/// the seller agreement and seller information). Have a legal professional review every text. By
/// `key` unless `order=desc`; paged with `limit` / `cursor`.
#[utoipa::path(
    get,
    path = "/templates",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_templates",
    security(("bearer" = [])),
    params(NaturalOrderPageQuery),
    responses(
        (status = 200, description = "One page of the templates, by key.", body = Page<TemplateSummary>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_templates(
    principal: Principal,
    Query(page): Query<NaturalOrderPageQuery>,
) -> Result<Page<TemplateSummary>, Error> {
    let page = PageRequest::try_from(page)?;
    crate::templates::summary_page(&principal, &page)
}

/// A draft page to make from a template.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = ContentTemplateDraftRequest)]
pub(crate) struct TemplateDraftRequest {
    /// `en` or `vi`; the tenant must write in that language.
    #[schema(example = "vi")]
    locale: String,
    /// Values for the template, e.g. `{"policy": {"return_days": 7}}`. They complete (and win
    /// over) the legal profile of the unit (`owner.*`, `contact.*`). Values only ever become text.
    #[serde(default)]
    #[schema(value_type = Object)]
    variables: Value,
    /// Parent page.
    parent_id: Option<Uuid>,
    /// Organization unit of the page (and of the legal profile used); default as for a new page.
    org_id: Option<Uuid>,
}

/// Make a draft page from a mandatory page template.
///
/// Requires `content.write` or `content.write_own`. Fills the template with the legal profile of
/// the unit (from `compliance`) and `variables`; a required value still missing gives
/// `content.template_variables_missing` listing them all. The result is a draft: have a legal
/// professional review it, publish it, then link it to the website
/// (`PUT /api/v1/compliance/websites/{id}/pages/{page_kind}`).
#[utoipa::path(
    post,
    path = "/templates/{key}/drafts",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "create_content_draft_from_template",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the template, e.g. `return_policy`.")),
    request_body = TemplateDraftRequest,
    responses(
        (status = 201, description = "The draft page.", body = Data<EntryResponse>, headers(("Location" = String, description = "Path of the entry."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such template; code `content.template_not_found`.", body = Problem),
        (status = 409, description = "The slug is taken; code `content.slug_taken`.", body = Problem),
        (status = 422, description = "Required template values are missing (`content.template_variables_missing`), or another value is invalid; every problem in `errors`.", body = Problem),
    )
)]
pub(crate) async fn draft_from_template(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    ValidJson(input): ValidJson<TemplateDraftRequest>,
) -> Result<Created<EntryResponse>, Error> {
    let draft = TemplateDraft {
        locale: input.locale,
        variables: input.variables,
        parent_id: input.parent_id,
        org_id: input.org_id,
    };
    let entry = service.draft_from_template(&tenant, &principal, &key, draft).await?;
    let location = format!("/content/entries/{}", entry.id);
    Ok(Created::new(location, tagged(&service, &tenant, entry).await?.data))
}
