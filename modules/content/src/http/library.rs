//! Endpoint quản lý danh mục / thẻ (`/terms`) và tác giả (`/authors`). Đọc cần
//! `content.read`; ghi cần `content.manage`.

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
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::{choice, page_request, present};
use crate::service::ContentService;
use crate::taxonomy::authors::{Author, AuthorInput};
use crate::taxonomy::terms::{Taxonomy, TermInput, TermWithTexts};

/// The name of a term in one language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct TermName {
    #[schema(example = "vi")]
    locale: String,
    name: String,
    /// Unique among the terms of the same taxonomy and language.
    slug: String,
    description: String,
    meta_title: String,
    meta_description: String,
}

/// A category or tag, with its names in every language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct TermResponse {
    id: Uuid,
    taxonomy: Taxonomy,
    /// Parent category; `null` at the top (always for tags).
    parent_id: Option<Uuid>,
    names: Vec<TermName>,
    /// How many entries (in any status) have this term. Only in the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entry_count: Option<u64>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TermWithTexts> for TermResponse {
    fn from(found: TermWithTexts) -> Self {
        TermResponse {
            taxonomy: if found.term.taxonomy == "category" {
                Taxonomy::Category
            } else {
                Taxonomy::Tag
            },
            id: found.term.id,
            entry_count: None,
            parent_id: found.term.parent_id,
            created_at: found.term.created_at,
            updated_at: found.term.updated_at,
            names: found
                .texts
                .into_iter()
                .map(|text| TermName {
                    locale: text.locale,
                    name: text.name,
                    slug: text.slug,
                    description: text.description,
                    meta_title: text.meta_title,
                    meta_description: text.meta_description,
                })
                .collect(),
        }
    }
}

/// The name of a term in one language, and its parent. Omitted fields stay as they are.
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TermFields {
    /// The language of `name`, `slug`, `description` and the meta fields; omitted: the
    /// default language. Needed to change those fields.
    locale: Option<String>,
    /// 1 to 200 characters.
    name: Option<String>,
    /// Omitted or empty: made from the name.
    slug: Option<String>,
    description: Option<String>,
    meta_title: Option<String>,
    meta_description: Option<String>,
    /// Parent category (categories only); `null` moves it to the top.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    parent_id: Option<Option<Uuid>>,
}

impl From<TermFields> for TermInput {
    fn from(input: TermFields) -> Self {
        TermInput {
            locale: input.locale,
            name: input.name,
            slug: input.slug,
            description: input.description,
            meta_title: input.meta_title,
            meta_description: input.meta_description,
            parent_id: input.parent_id,
        }
    }
}

/// A new category or tag.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct CreateTermRequest {
    taxonomy: Taxonomy,
    #[serde(flatten)]
    fields: TermFields,
}

/// Which terms to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct TermsQuery {
    /// `category` or `tag`.
    #[param(value_type = Option<Taxonomy>, inline)]
    taxonomy: Option<String>,
    /// Only terms whose name (in any language) contains this text, ignoring case and accents.
    q: Option<String>,
}

/// List categories and tags.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/terms",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_terms",
    security(("bearer" = [])),
    params(TermsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of terms.", body = Page<TermResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_terms(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Query(query): Query<TermsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<TermResponse>, Error> {
    let mut problems = Vec::new();
    let taxonomy =
        choice(query.taxonomy.as_deref(), "taxonomy", "`category`, `tag`", &mut problems);
    let page = page_request(paging, problems)?;
    let page = service.list_terms(&principal, taxonomy, query.q.as_deref(), &page).await?;
    let ids: Vec<Uuid> = page.data.iter().map(|found| found.term.id).collect();
    let counts = service.term_entry_counts(&ids).await?;
    Ok(page.map(|found| {
        let count = counts.get(&found.term.id).copied().unwrap_or(0);
        TermResponse { entry_count: Some(count), ..TermResponse::from(found) }
    }))
}

/// Create a category or tag.
///
/// Requires `content.manage`. Give its name in one language; add other languages with
/// `PATCH`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/terms",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "create_content_term",
    security(("bearer" = [])),
    request_body = CreateTermRequest,
    responses(
        (status = 201, description = "Created.", body = Data<TermResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The slug is taken; code `content.slug_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_term(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    ValidJson(input): ValidJson<CreateTermRequest>,
) -> Result<Created<TermResponse>, Error> {
    let found = service.create_term(&principal, input.taxonomy, input.fields.into()).await?;
    Ok(Created::new(format!("/content/terms/{}", found.term.id), found.into()))
}

/// Get a category or tag.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/terms/{id}",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_term",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the term.")),
    responses(
        (status = 200, description = "The term.", body = Data<TermResponse>, headers(("ETag" = String, description = "Version of the term."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No term has this id; code `content.term_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_term(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<TermResponse>, Error> {
    let found = service.get_term(&principal, id).await?;
    Ok(Tagged { version: found.term.updated_at, data: found.into() })
}

/// Change a category or tag.
///
/// Requires `content.manage`. Sets its name in one language (adding the language if new)
/// and its parent. `If-Match` is required.
#[utoipa::path(
    patch,
    path = "/terms/{id}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "update_content_term",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the term."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = TermFields,
    responses(
        (status = 200, description = "The term after the change.", body = Data<TermResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No term has this id; code `content.term_not_found`.", body = Problem),
        (status = 409, description = "The slug is taken; code `content.slug_taken`.", body = Problem),
        (status = 412, description = "The term changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid, or the parent would make a loop (`term_cycle`); see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_term(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<TermFields>,
) -> Result<Tagged<TermResponse>, Error> {
    let expected = if_match.required()?;
    let found = service.update_term(&principal, id, input.into(), expected).await?;
    Ok(Tagged { version: found.term.updated_at, data: found.into() })
}

/// Delete a category or tag.
///
/// Requires `content.manage`. It is removed from every entry. A category with
/// subcategories cannot be deleted.
#[utoipa::path(
    delete,
    path = "/terms/{id}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "delete_content_term",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the term.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No term has this id; code `content.term_not_found`.", body = Problem),
        (status = 409, description = "The category has subcategories; code `content.term_has_children`.", body = Problem),
    )
)]
pub(crate) async fn delete_term(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_term(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// An author profile.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct AuthorResponse {
    id: Uuid,
    /// The user account behind the profile, if any.
    user_id: Option<Uuid>,
    #[schema(example = "Ada Lovelace")]
    display_name: String,
    /// Unique.
    slug: String,
    bio: String,
    /// Picture (a media item id).
    avatar_media_id: Option<Uuid>,
    website_url: String,
    /// How many entries (in any status) list this author. Only in the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entry_count: Option<u64>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<Author> for AuthorResponse {
    fn from(row: Author) -> Self {
        AuthorResponse {
            id: row.id,
            user_id: row.user_id,
            display_name: row.display_name,
            entry_count: None,
            slug: row.slug,
            bio: row.bio,
            avatar_media_id: row.avatar_media_id,
            website_url: row.website_url,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Fields of an author profile. Omitted fields stay as they are.
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthorFields {
    /// Link the profile to a user account; `null` unlinks it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    user_id: Option<Option<Uuid>>,
    /// 1 to 200 characters.
    display_name: Option<String>,
    /// Omitted or empty: made from the name.
    slug: Option<String>,
    /// Up to 5000 characters.
    bio: Option<String>,
    /// Picture (a media item id); `null` removes it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    avatar_media_id: Option<Option<Uuid>>,
    /// An `https://` address.
    website_url: Option<String>,
}

impl From<AuthorFields> for AuthorInput {
    fn from(input: AuthorFields) -> Self {
        AuthorInput {
            user_id: input.user_id,
            display_name: input.display_name,
            slug: input.slug,
            bio: input.bio,
            avatar_media_id: input.avatar_media_id,
            website_url: input.website_url,
        }
    }
}

/// List author profiles.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/authors",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "list_content_authors",
    security(("bearer" = [])),
    params(AuthorsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of authors.", body = Page<AuthorResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_authors(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Query(search): Query<AuthorsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<AuthorResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let page = service.list_authors(&principal, search.q.as_deref(), &page).await?;
    let ids: Vec<Uuid> = page.data.iter().map(|author| author.id).collect();
    let counts = service.author_entry_counts(&ids).await?;
    Ok(page.map(|author| {
        let count = counts.get(&author.id).copied().unwrap_or(0);
        AuthorResponse { entry_count: Some(count), ..AuthorResponse::from(author) }
    }))
}

/// Which author profiles to list.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AuthorsQuery {
    /// Only profiles whose display name contains this text, ignoring case and accents.
    q: Option<String>,
}

/// Create an author profile.
///
/// Requires `content.manage`. Retry safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/authors",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "create_content_author",
    security(("bearer" = [])),
    request_body = AuthorFields,
    responses(
        (status = 201, description = "Created.", body = Data<AuthorResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 409, description = "The slug is taken; code `content.slug_taken`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_author(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    ValidJson(input): ValidJson<AuthorFields>,
) -> Result<Created<AuthorResponse>, Error> {
    let row = service.create_author(&principal, input.into()).await?;
    Ok(Created::new(format!("/content/authors/{}", row.id), row.into()))
}

/// Get an author profile.
///
/// Requires `content.read`.
#[utoipa::path(
    get,
    path = "/authors/{id}",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_author",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the author.")),
    responses(
        (status = 200, description = "The author.", body = Data<AuthorResponse>, headers(("ETag" = String, description = "Version of the profile."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No author has this id; code `content.author_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_author(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<AuthorResponse>, Error> {
    let row = service.get_author(&principal, id).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// Change an author profile.
///
/// Requires `content.manage`. `If-Match` is required.
#[utoipa::path(
    patch,
    path = "/authors/{id}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "update_content_author",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the author."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = AuthorFields,
    responses(
        (status = 200, description = "The author after the change.", body = Data<AuthorResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No author has this id; code `content.author_not_found`.", body = Problem),
        (status = 409, description = "The slug is taken; code `content.slug_taken`.", body = Problem),
        (status = 412, description = "The profile changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_author(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<AuthorFields>,
) -> Result<Tagged<AuthorResponse>, Error> {
    let expected = if_match.required()?;
    let row = service.update_author(&principal, id, input.into(), expected).await?;
    Ok(Tagged { version: row.updated_at, data: row.into() })
}

/// Delete an author profile.
///
/// Requires `content.manage`. It is removed from every entry; the entries stay.
#[utoipa::path(
    delete,
    path = "/authors/{id}",
    extensions(("x-permissions" = json!(["content.manage"]))),
    operation_id = "delete_content_author",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the author.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No author has this id; code `content.author_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_author(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_author(&principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
