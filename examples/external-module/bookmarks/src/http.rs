//! REST endpoints of the module, under `/api/v1/bookmarks` (the module declares only the tail of
//! the path). A handler does three things: take the input, check the permission, call the service.
//! Routes are registered through `routes!`, which puts them into the OpenAPI document; the `///`
//! lines and every `description` are copied there, so they are written for the API's callers.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{BOOKMARKS_READ, BOOKMARKS_WRITE};
use crate::service::{Bookmark, BookmarkService, NewBookmark};

pub(crate) fn routes(service: Arc<BookmarkService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_bookmarks, create_bookmark))
        .routes(routes!(get_bookmark, delete_bookmark))
        .with_state(service)
}

/// A bookmark.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub(crate) struct BookmarkResponse {
    id: Uuid,
    #[schema(example = "https://www.rust-lang.org/")]
    url: String,
    #[schema(example = "Rust")]
    title: String,
    created_at: Timestamp,
}

impl From<Bookmark> for BookmarkResponse {
    fn from(bookmark: Bookmark) -> Self {
        BookmarkResponse {
            id: bookmark.id,
            url: bookmark.url,
            title: bookmark.title,
            created_at: bookmark.created_at,
        }
    }
}

/// A new bookmark.
// The service checks the fields (and reports every problem at once), so there is no
// `#[validate]` here: one place checks each field.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBookmarkRequest {
    /// The link: an `http://` or `https://` address, up to 2000 characters. You keep each link once.
    #[schema(example = "https://www.rust-lang.org/")]
    url: String,
    /// Title, 1 to 200 characters.
    #[schema(example = "Rust")]
    title: String,
}

impl From<CreateBookmarkRequest> for NewBookmark {
    fn from(input: CreateBookmarkRequest) -> Self {
        NewBookmark { url: input.url, title: input.title }
    }
}

/// List your bookmarks.
///
/// Requires `bookmarks.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/",
    extensions(("x-permissions" = json!(["bookmarks.read"]))),
    operation_id = "list_bookmarks",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your bookmarks.", body = Page<BookmarkResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `bookmarks.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_bookmarks(
    State(service): State<Arc<BookmarkService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<PageQuery>,
) -> Result<Page<BookmarkResponse>, Error> {
    principal.require(&BOOKMARKS_READ)?;
    let page = service.list(&tenant, &principal, &PageRequest::try_from(query)?).await?;
    Ok(page.map(BookmarkResponse::from))
}

/// Create a bookmark.
///
/// Requires `bookmarks.write`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/",
    extensions(("x-permissions" = json!(["bookmarks.write"]))),
    operation_id = "create_bookmark",
    security(("bearer" = [])),
    request_body = CreateBookmarkRequest,
    responses(
        (status = 201, description = "Created.", body = Data<BookmarkResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `bookmarks.write`; code `permission_denied`.", body = Problem),
        (status = 409, description = "You already keep this link; code `bookmarks.url_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
async fn create_bookmark(
    State(service): State<Arc<BookmarkService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateBookmarkRequest>,
) -> Result<Created<BookmarkResponse>, Error> {
    principal.require(&BOOKMARKS_WRITE)?;
    let bookmark = service.create(&tenant, &principal, input.into()).await?;
    Ok(Created::new(format!("/bookmarks/{}", bookmark.id), bookmark.into()))
}

/// Get one of your bookmarks.
///
/// Requires `bookmarks.read`.
#[utoipa::path(
    get,
    path = "/{id}",
    extensions(("x-permissions" = json!(["bookmarks.read"]))),
    operation_id = "get_bookmark",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bookmark.")),
    responses(
        (status = 200, description = "The bookmark.", body = Data<BookmarkResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `bookmarks.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "You have no bookmark with this id; code `bookmarks.bookmark_not_found`.", body = Problem),
    )
)]
async fn get_bookmark(
    State(service): State<Arc<BookmarkService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BookmarkResponse>, Error> {
    principal.require(&BOOKMARKS_READ)?;
    let bookmark = service.get(&tenant, &principal, id).await?;
    Ok(Data { data: bookmark.into() })
}

/// Delete one of your bookmarks.
///
/// Requires `bookmarks.write`. It cannot be restored.
#[utoipa::path(
    delete,
    path = "/{id}",
    extensions(("x-permissions" = json!(["bookmarks.write"]))),
    operation_id = "delete_bookmark",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the bookmark.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `bookmarks.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "You have no bookmark with this id; code `bookmarks.bookmark_not_found`.", body = Problem),
    )
)]
async fn delete_bookmark(
    State(service): State<Arc<BookmarkService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&BOOKMARKS_WRITE)?;
    service.delete(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
