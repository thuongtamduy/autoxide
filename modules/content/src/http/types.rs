//! Loại nội dung tự định nghĩa (bước 67, D-235): quản lý loại và đọc công khai mục của loại. Mục của
//! loại tạo / sửa / đăng như mọi mục (`POST /entries` với `kind = custom` và `type_id`). Logic ở `types/`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use autoxide_kernel::time::Timestamp;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::page_request;
use super::public::{PublicEntryResponse, public_cache, views};
use crate::service::ContentService;
use crate::types::{ContentType, TypeInput};

/// A custom content type of the tenant (team, projects, branches…).
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ContentTypeResponse {
    id: Uuid,
    /// `snake_case`; cannot change once the type has entries. Its own fields are custom fields of
    /// target `content.type.<key>` (`/api/v1/kernel/custom-fields`).
    key: String,
    /// `{"one": {"en": "Project"}, "many": {"en": "Projects"}}`.
    #[schema(value_type = Object)]
    labels: Value,
    icon: String,
    /// Entries have their own page at `/<path_prefix>/<slug>`.
    routable: bool,
    path_prefix: Option<String>,
    /// Body of the entries: `none`, `rich` or `layout`.
    body: String,
    has_seo: bool,
    /// Entries are listed publicly at `GET /public/types/{key}/entries`.
    listable: bool,
    /// Switched off: entries stop being public (nothing is lost).
    active: bool,
    org_id: Uuid,
    /// Only for a type with pages: the block layout every entry page of the type uses (blocks
    /// `content.entry_field` show the entry's own fields). `null`: none; the website draws entries
    /// from their own body.
    #[schema(value_type = Option<Object>)]
    detail_layout: Option<Value>,
    #[schema(value_type = String, format = DateTime)]
    updated_at: Timestamp,
}

fn response(row: ContentType) -> ContentTypeResponse {
    ContentTypeResponse {
        labels: serde_json::from_str(&row.labels).unwrap_or(Value::Null),
        detail_layout: row
            .detail_layout
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok()),
        id: row.id,
        key: row.key,
        icon: row.icon,
        routable: row.routable,
        path_prefix: row.path_prefix,
        body: row.body,
        has_seo: row.has_seo,
        listable: row.listable,
        active: row.active,
        org_id: row.org_id,
        updated_at: row.updated_at,
    }
}

/// A content type to create, or the fields to change.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContentTypeRequest {
    /// Required on create: 1 to 64 lower-case letters, digits or `_`.
    key: Option<String>,
    /// Required on create: `{"one": {language: text}, "many": {language: text}}`.
    #[schema(value_type = Option<Object>)]
    labels: Option<Value>,
    /// `people`, `briefcase`, `map_pin`, `star`, `question`, `calendar`, `image`, `document`, `tag`,
    /// `building`, `gift` or `chat`.
    icon: Option<String>,
    routable: Option<bool>,
    /// Address segment of the entries' pages, e.g. `du-an`; required when `routable`.
    path_prefix: Option<String>,
    /// `none`, `rich` (default) or `layout`.
    body: Option<String>,
    has_seo: Option<bool>,
    listable: Option<bool>,
    active: Option<bool>,
    /// The website (organization unit), create only; left out: the whole tenant.
    org_id: Option<Uuid>,
    /// Only with `routable`: the block layout (`{"type": "layout", "schema": 1, "sections": […]}`)
    /// of every entry page of the type, checked like a page layout; blocks
    /// `content.entry_field {"field": …}` show a field of the entry being viewed. `null` removes it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Object>)]
    detail_layout: Option<Option<Value>>,
}

/// Trường có gửi lên hay không: thiếu → `None`, `null` → `Some(None)` (bỏ giá trị).
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl From<ContentTypeRequest> for TypeInput {
    fn from(input: ContentTypeRequest) -> Self {
        TypeInput {
            key: input.key,
            labels: input.labels,
            icon: input.icon,
            routable: input.routable,
            path_prefix: input.path_prefix,
            body: input.body,
            has_seo: input.has_seo,
            listable: input.listable,
            active: input.active,
            org_id: input.org_id,
            detail_layout: input.detail_layout,
        }
    }
}

/// List custom content types.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/types",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_types",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "A page of content types.", body = Page<ContentTypeResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_types(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ContentTypeResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    Ok(service.list_types(&tenant, &principal, &page).await?.map(response))
}

/// Create a custom content type.
///
/// Requires `content.types.manage`. Then define its fields as custom fields of target
/// `content.type.<key>` and create entries with `POST /entries` (`kind: "custom"`, `type_id`).
#[utoipa::path(
    post,
    path = "/types",
    extensions(("x-permissions" = json!(["content.types.manage"]))),
    operation_id = "create_content_type",
    security(("bearer" = [])),
    request_body = ContentTypeRequest,
    responses(
        (status = 201, description = "Created.", body = autoxide_kernel::response::Data<ContentTypeResponse>, headers(("Location" = String, description = "Address of the new type."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.types.manage` (code `permission_denied`), or the plan has no `content.custom_types` (code `feature_not_enabled`).", body = Problem),
        (status = 409, description = "The key or address prefix is taken (`content.type_key_taken`, `content.path_prefix_taken`), or the tenant has the most types allowed (`content.types_limit` for the server's ceiling, `limit_exceeded` for the plan's).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_type(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<ContentTypeRequest>,
) -> Result<Created<ContentTypeResponse>, Error> {
    let row = service.create_type(&tenant, &principal, input.into()).await?;
    let location = format!("/api/v1/content/types/{}", row.key);
    Ok(Created::new(location, response(row)))
}

/// Get a custom content type.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/types/{key}",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_type",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the content type.")),
    responses(
        (status = 200, description = "The content type.", body = autoxide_kernel::response::Data<ContentTypeResponse>, headers(("ETag" = String, description = "Version: send it back in `If-Match` when changing it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No content type has this key; code `content.type_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_type(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
) -> Result<Tagged<ContentTypeResponse>, Error> {
    let row = service.content_type(&tenant, &principal, &key).await?;
    Ok(Tagged { version: row.updated_at, data: response(row) })
}

/// Change a custom content type.
///
/// Requires `content.types.manage`; `If-Match` is required. Only the fields sent change. The key
/// cannot change once the type has entries; switching the type off (`active: false`) takes its
/// entries off the website without losing them.
#[utoipa::path(
    patch,
    path = "/types/{key}",
    extensions(("x-permissions" = json!(["content.types.manage"]))),
    operation_id = "update_content_type",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the content type."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`). Otherwise `412`.")),
    request_body = ContentTypeRequest,
    responses(
        (status = 200, description = "The content type after the change.", body = autoxide_kernel::response::Data<ContentTypeResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.types.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No content type has this key; code `content.type_not_found`.", body = Problem),
        (status = 409, description = "The key cannot change (`content.type_key_locked`), or the new key or prefix is taken.", body = Problem),
        (status = 412, description = "It changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_type(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ContentTypeRequest>,
) -> Result<Tagged<ContentTypeResponse>, Error> {
    let expected = if_match.required()?;
    let input = TypeInput { org_id: None, ..input.into() };
    let row = service.update_type(&tenant, &principal, &key, input, expected).await?;
    Ok(Tagged { version: row.updated_at, data: response(row) })
}

/// Delete a custom content type.
///
/// Requires `content.types.manage`. A type that still has entries is not deleted (`409`).
#[utoipa::path(
    delete,
    path = "/types/{key}",
    extensions(("x-permissions" = json!(["content.types.manage"]))),
    operation_id = "delete_content_type",
    security(("bearer" = [])),
    params(("key" = String, Path, description = "Key of the content type.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.types.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No content type has this key; code `content.type_not_found`.", body = Problem),
        (status = 409, description = "The type still has entries; code `content.type_in_use` with `count`.", body = Problem),
    )
)]
pub(crate) async fn delete_type(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(key): Path<String>,
) -> Result<StatusCode, Error> {
    service.delete_type(&tenant, &principal, &key).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The language of the list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TypeEntriesQuery {
    /// A language of the website; left out: its default.
    locale: Option<String>,
}

/// List the published entries of a content type (public).
///
/// No sign-in needed. Only types switched on with `listable`; newest first. Each entry is like
/// `GET /public/resolve` gives it (with the type's `public` custom fields). Filter on the published
/// values of filterable public fields with `cf.<key>=<eq|in|gte|lte|prefix>:<value>`.
/// `Cache-Control: no-cache`.
#[utoipa::path(
    get,
    path = "/public/types/{key}/entries",
    extensions(("x-public" = json!(true))),
    operation_id = "list_content_public_type_entries",
    security((), ("bearer" = [])),
    params(("key" = String, Path, description = "Key of the content type."), TypeEntriesQuery, SiteQuery, PageQuery),
    responses(
        (status = 200, description = "A page of published entries.", body = Page<PublicEntryResponse>),
        (status = 404, description = "No listable content type has this key; code `content.type_not_found`.", body = Problem),
        (status = 422, description = "The language, a paging value or a `cf.<key>` filter is invalid (unknown or not public: `unknown`; not filterable: code `kernel.custom_field_not_filterable`); see `errors`.", body = Problem),
    )
)]
pub(crate) async fn public_type_entries(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    site: Site,
    Path(key): Path<String>,
    Query(query): Query<TypeEntriesQuery>,
    Query(paging): Query<PageQuery>,
    Query(raw): Query<std::collections::HashMap<String, String>>,
) -> Result<Response, Error> {
    let mut problems = Vec::new();
    let languages = service.site_languages(&tenant, &site).await?;
    let locale = service.check_locale(&languages, query.locale.as_deref(), "locale", &mut problems);
    let page = page_request(paging, problems)?;
    let mut filters: Vec<(String, String)> = raw
        .into_iter()
        .filter_map(|(name, value)| name.strip_prefix("cf.").map(|key| (key.to_owned(), value)))
        .collect();
    filters.sort();
    let (_, found) =
        service.public_type_entries(&tenant, &site, &key, &locale, &filters, &page).await?;
    let page = views(&service, &tenant, found).await?;
    Ok(public_cache(page.into_response()))
}

/// What the new content type is for.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuggestTypeRequest {
    /// Required: in the owner's words, 1 to 2000 characters, e.g. "a page introducing our team,
    /// one card per person".
    #[serde(default)]
    description: String,
    /// Required: language of the labels, e.g. `vi`.
    #[serde(default)]
    locale: String,
}

/// A draft definition of a content type. Nothing is saved: review it, then create it.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct TypeSuggestionResponse {
    /// Body for `POST /types`.
    #[schema(value_type = Object)]
    content_type: Value,
    /// Bodies for `POST /api/v1/kernel/custom-fields`, one per field (target
    /// `content.type.<key>`), after the type is created.
    #[schema(value_type = Vec<Object>)]
    fields: Vec<Value>,
    /// The model that wrote the draft.
    model: String,
    /// Always `true`: a person reviews the draft before creating anything.
    draft: bool,
}

/// Suggest a content type with AI.
///
/// Requires `content.types.manage`, the plan feature `content.ai_suggestions` and AI switched on
/// for the server. From a description, the model drafts the key, labels, icon, pages and fields of
/// a new content type. Nothing is saved; each call uses AI tokens.
#[utoipa::path(
    post,
    path = "/types/ai/suggest",
    extensions(("x-permissions" = json!(["content.types.manage"]))),
    operation_id = "suggest_content_type",
    security(("bearer" = [])),
    request_body = SuggestTypeRequest,
    responses(
        (status = 200, description = "A draft definition.", body = autoxide_kernel::response::Data<TypeSuggestionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.types.manage` (code `permission_denied`), or the plan has no `content.ai_suggestions` (code `feature_not_enabled`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 503, description = "AI is off (code `ai.disabled`), misconfigured (code `ai.misconfigured`), the provider is busy (code `ai.provider_unavailable`, maybe with `Retry-After`), or its answer had the wrong shape twice (code `ai.output_invalid`).", body = Problem),
    )
)]
pub(crate) async fn suggest_type(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<SuggestTypeRequest>,
) -> Result<autoxide_kernel::response::Data<TypeSuggestionResponse>, Error> {
    let found =
        service.suggest_type(&tenant, &principal, &input.description, &input.locale).await?;
    Ok(autoxide_kernel::response::Data {
        data: TypeSuggestionResponse {
            content_type: found.content_type,
            fields: found.fields,
            model: found.model,
            draft: true,
        },
    })
}
