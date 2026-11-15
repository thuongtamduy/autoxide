//! Endpoint quản lý danh sách (`/lists`, `/lists/{id}`). Đọc cần `newsletter.read`; ghi cần
//! `newsletter.manage`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::version::{IfMatch, Tagged};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::{page_request, present};
use crate::lists::{List, ListInput, NewPosts, SubscriptionCounts};
use crate::service::NewsletterService;
use crate::text::{Locale, Localized};

/// A newsletter list, as admins see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct ListResponse {
    pub id: Uuid,
    /// Unique; websites send it to subscribe.
    #[schema(example = "weekly")]
    pub key: String,
    pub name: Localized,
    pub description: Localized,
    /// Public sign-ups get a confirmation mail and receive nothing until they open its link.
    pub double_opt_in: bool,
    /// Shown by `GET /api/v1/newsletter/public/lists` and open to public sign-up.
    pub public: bool,
    /// What happens when the `content` module publishes a post.
    pub new_posts: NewPosts,
    /// Only posts in this language make campaigns; `null`: any language.
    pub new_post_locale: Option<Locale>,
    /// Subscriptions by status.
    pub subscriptions: SubscriptionCounts,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ListResponse {
    pub(crate) fn new(list: List, subscriptions: SubscriptionCounts) -> Self {
        ListResponse {
            new_posts: list.new_posts(),
            new_post_locale: list.new_post_locale(),
            id: list.row.id,
            key: list.row.key,
            name: list.name,
            description: list.description,
            double_opt_in: list.row.double_opt_in,
            public: list.row.public,
            subscriptions,
            created_at: list.row.created_at,
            updated_at: list.row.updated_at,
        }
    }
}

/// A new list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateListRequest {
    /// Lower-case letters and digits in groups joined by `-`, at most 50 characters; unique.
    #[schema(example = "weekly")]
    key: String,
    /// Up to 200 characters per language; at least one language.
    name: Localized,
    /// Up to 1000 characters per language.
    description: Option<Localized>,
    /// Omitted: `true`.
    double_opt_in: Option<bool>,
    /// Omitted: `false` (not open to public sign-up).
    public: Option<bool>,
    /// Omitted: `off`. `send` also needs the permission `newsletter.send`.
    new_posts: Option<NewPosts>,
    /// Omitted: posts in any language.
    new_post_locale: Option<Locale>,
}

/// Changes to a list. Omitted fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateListRequest {
    key: Option<String>,
    name: Option<Localized>,
    description: Option<Localized>,
    double_opt_in: Option<bool>,
    public: Option<bool>,
    /// Setting `send` also needs the permission `newsletter.send`.
    new_posts: Option<NewPosts>,
    /// `null`: posts in any language.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Locale>, nullable)]
    new_post_locale: Option<Option<Locale>>,
}

/// List the newsletter lists.
///
/// Requires `newsletter.read`. Newest first, with subscription counts; follow
/// `pagination.next_cursor` for more.
#[utoipa::path(
    get,
    path = "/lists",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "list_newsletter_lists",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of lists.", body = Page<ListResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_lists(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Query(paging): Query<PageQuery>,
) -> Result<Page<ListResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let found = service.list_lists(&principal, &page).await?;
    let ids: Vec<Uuid> = found.data.iter().map(|list| list.row.id).collect();
    let mut counts = service.subscription_counts(&ids).await?;
    Ok(found.map(|list| {
        let count = counts.remove(&list.row.id).unwrap_or_default();
        ListResponse::new(list, count)
    }))
}

/// Create a list.
///
/// Requires `newsletter.manage`, and also `newsletter.send` for `new_posts: send` (new posts
/// are then sent without review). Every problem is reported at once in `errors`. Retry safely
/// with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/lists",
    extensions(("x-permissions" = json!(["newsletter.manage", "newsletter.send"]))),
    operation_id = "create_newsletter_list",
    security(("bearer" = [])),
    request_body = CreateListRequest,
    responses(
        (status = 201, description = "Created.", body = Data<ListResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage` (or `newsletter.send`, needed for `new_posts: send`); code `permission_denied`.", body = Problem),
        (status = 409, description = "The key is taken; code `newsletter.list_key_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_list(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    ValidJson(input): ValidJson<CreateListRequest>,
) -> Result<Created<ListResponse>, Error> {
    let input = ListInput {
        key: Some(input.key),
        name: Some(input.name),
        description: input.description,
        double_opt_in: input.double_opt_in,
        public: input.public,
        new_posts: input.new_posts,
        new_post_locale: input.new_post_locale.map(Some),
    };
    let list = service.create_list(&principal, input).await?;
    Ok(Created::new(
        format!("/newsletter/lists/{}", list.row.id),
        ListResponse::new(list, SubscriptionCounts::default()),
    ))
}

/// Get a list.
///
/// Requires `newsletter.read`.
#[utoipa::path(
    get,
    path = "/lists/{id}",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "get_newsletter_list",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the list.")),
    responses(
        (status = 200, description = "The list.", body = Data<ListResponse>, headers(("ETag" = String, description = "Version of the list; send it in `If-Match` to change it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No list has this id; code `newsletter.list_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_list(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<ListResponse>, Error> {
    let list = service.get_list(&principal, id).await?;
    let count = service.subscription_counts(&[id]).await?.remove(&id).unwrap_or_default();
    Ok(Tagged { version: list.row.updated_at, data: ListResponse::new(list, count) })
}

/// Change a list.
///
/// Requires `newsletter.manage`, and also `newsletter.send` to set `new_posts: send` (new
/// posts are then sent without review). `If-Match` is required.
#[utoipa::path(
    patch,
    path = "/lists/{id}",
    extensions(("x-permissions" = json!(["newsletter.manage", "newsletter.send"]))),
    operation_id = "update_newsletter_list",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the list."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateListRequest,
    responses(
        (status = 200, description = "The list after the change.", body = Data<ListResponse>, headers(("ETag" = String, description = "New version of the list."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage` (or `newsletter.send`, needed for `new_posts: send`); code `permission_denied`.", body = Problem),
        (status = 404, description = "No list has this id; code `newsletter.list_not_found`.", body = Problem),
        (status = 409, description = "The key is taken; code `newsletter.list_key_taken`.", body = Problem),
        (status = 412, description = "The list changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_list(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateListRequest>,
) -> Result<Tagged<ListResponse>, Error> {
    let expected = if_match.required()?;
    let input = ListInput {
        key: input.key,
        name: input.name,
        description: input.description,
        double_opt_in: input.double_opt_in,
        public: input.public,
        new_posts: input.new_posts,
        new_post_locale: input.new_post_locale,
    };
    let list = service.update_list(&principal, id, input, expected).await?;
    let count = service.subscription_counts(&[id]).await?.remove(&id).unwrap_or_default();
    Ok(Tagged { version: list.row.updated_at, data: ListResponse::new(list, count) })
}

/// Delete a list.
///
/// Requires `newsletter.manage`. `If-Match` is required. Deletes every subscription to the
/// list. Refused while the list has a draft, scheduled, sending or paused campaign.
#[utoipa::path(
    delete,
    path = "/lists/{id}",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "delete_newsletter_list",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the list."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    responses(
        (status = 204, description = "Deleted, with its subscriptions."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No list has this id; code `newsletter.list_not_found`.", body = Problem),
        (status = 409, description = "The list has unfinished campaigns; code `newsletter.list_in_use`.", body = Problem),
        (status = 412, description = "The list changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn delete_list(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<StatusCode, Error> {
    let expected = if_match.required()?;
    service.delete_list(&principal, id, expected).await?;
    Ok(StatusCode::NO_CONTENT)
}
