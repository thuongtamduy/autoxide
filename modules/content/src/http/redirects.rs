//! Endpoint quản lý chuyển hướng: `/api/v1/content/redirects`. Cần `content.manage`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;
use autoxide_spike_macros::Schema as ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::page_request;
use crate::publishing::redirects::{Redirect, RedirectInput};
use crate::service::ContentService;

/// A redirect from an old address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct RedirectResponse {
    id: Uuid,
    locale: String,
    #[schema(example = "/old-page")]
    from_path: String,
    /// Target entry (followed by id, so it keeps working when that entry moves).
    to_entry_id: Option<Uuid>,
    /// Target address (a path in the site, or a full URL).
    to_url: Option<String>,
    /// 301, 302, 307, 308, or 410 (gone).
    #[schema(example = 301)]
    status_code: i32,
    /// `auto` (made when a published address changed), `gone` (made when an entry was taken
    /// down for good) or `manual`.
    source: String,
    created_by: Option<Uuid>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl From<Redirect> for RedirectResponse {
    fn from(row: Redirect) -> Self {
        RedirectResponse {
            id: row.id,
            locale: row.locale,
            from_path: row.from_path,
            to_entry_id: row.to_entry_id,
            to_url: row.to_url,
            status_code: row.status_code,
            source: row.source,
            created_by: row.created_by,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A redirect to create or set.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RedirectRequest {
    /// Language; omitted: the default language.
    locale: Option<String>,
    /// The old address, a path starting with `/`.
    from_path: String,
    /// Target entry; give this or `to_url` (neither for 410).
    to_entry_id: Option<Uuid>,
    /// Target address: a path starting with `/` or an `https://` URL.
    to_url: Option<String>,
    /// 301 (default), 302, 307, 308, or 410.
    #[serde(default = "permanent")]
    status_code: i32,
}

fn permanent() -> i32 {
    301
}

impl From<RedirectRequest> for RedirectInput {
    fn from(input: RedirectRequest) -> Self {
        RedirectInput {
            locale: input.locale,
            from_path: input.from_path,
            to_entry_id: input.to_entry_id,
            to_url: input.to_url,
            status_code: input.status_code,
        }
    }
}

/// Which redirects to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct RedirectsQuery {
    /// Only this language.
    locale: Option<String>,
    /// Only `auto`, `gone` or `manual` redirects.
    source: Option<String>,
    /// Only redirects whose old path or target address contains this text, in any case and with
    /// or without accents.
    q: Option<String>,
}

/// List redirects.
///
/// Requires `content.manage`.
#[utoipa::path(
    get,
    path = "/redirects",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "list_content_redirects",
    security(("bearer" = [])),
    params(RedirectsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of redirects.", body = Page<RedirectResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_redirects(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<RedirectsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<RedirectResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let found = service
        .list_redirects(
            &tenant,
            &principal,
            query.locale.as_deref(),
            query.source.as_deref(),
            query.q.as_deref(),
            &page,
        )
        .await?;
    Ok(found.map(RedirectResponse::from))
}

/// Create a redirect.
///
/// Requires `content.manage`. Refused when it would lead back to its own address
/// (`content.redirect_loop`) or when a published entry lives at `from_path`. Retry safely with
/// an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/redirects",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "create_content_redirect",
    security(("bearer" = [])),
    request_body = RedirectRequest,
    responses(
        (status = 201, description = "Created.", body = Data<RedirectResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "A loop (code `content.redirect_loop`), a published entry at that address (code `content.path_taken`), or a redirect from that address already (code `content.redirect_exists`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_redirect(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<RedirectRequest>,
) -> Result<Created<RedirectResponse>, Error> {
    let row = service.create_redirect(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/content/redirects/{}", row.id), row.into()))
}

/// Get a redirect.
///
/// Requires `content.manage`.
#[utoipa::path(
    get,
    path = "/redirects/{id}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "get_content_redirect",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the redirect.")),
    responses(
        (status = 200, description = "The redirect.", body = Data<RedirectResponse>, headers(("ETag" = String, description = "Version of the redirect."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No redirect has this id; code `content.redirect_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_redirect(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<RedirectResponse>, Error> {
    let row = service.get_redirect(&tenant, &principal, id).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// Change a redirect.
///
/// Requires `content.manage`. Sends the whole redirect; it becomes a manual one. `If-Match`
/// is required.
#[utoipa::path(
    patch,
    path = "/redirects/{id}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "update_content_redirect",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the redirect."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = RedirectRequest,
    responses(
        (status = 200, description = "The redirect after the change.", body = Data<RedirectResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No redirect has this id; code `content.redirect_not_found`.", body = Problem),
        (status = 409, description = "A loop, a published entry at that address, or another redirect from it; see the code.", body = Problem),
        (status = 412, description = "The redirect changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_redirect(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<RedirectRequest>,
) -> Result<Tagged<RedirectResponse>, Error> {
    let expected = if_match.required()?;
    let row = service.update_redirect(&tenant, &principal, id, input.into(), expected).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// Delete a redirect.
///
/// Requires `content.manage`. The old address then answers 404.
#[utoipa::path(
    delete,
    path = "/redirects/{id}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "delete_content_redirect",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the redirect.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No redirect has this id; code `content.redirect_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_redirect(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_redirect(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
