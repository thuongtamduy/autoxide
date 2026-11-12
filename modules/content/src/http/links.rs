//! Bản dịch của mục, và term / tác giả gắn vào mục (có hiệu lực ngay, không qua nháp).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Created, Data, Problem};
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::entries::{EntryFieldsInput, EntryResponse, tagged};
use crate::service::ContentService;

/// A new translation.
#[derive(Debug, Deserialize, Validate, ToSchema)]
pub(crate) struct TranslationRequest {
    /// The language of the translation.
    #[schema(example = "vi")]
    locale: String,
    /// Fields of the translation; omitted ones are copied from the entry.
    #[serde(flatten)]
    fields: EntryFieldsInput,
}

/// The categories and tags of an entry.
#[derive(Debug, Serialize, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryTerms {
    /// In order; the first category is the main one. At most 50.
    term_ids: Vec<Uuid>,
}

/// The authors of an entry.
#[derive(Debug, Serialize, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryAuthors {
    /// In order; the first is the main one. At most 20.
    author_ids: Vec<Uuid>,
}

/// Translate a post or page.
///
/// Requires `content.write`, or `content.write_own` when you created the original. Creates a new
/// draft entry in `locale`, linked to this one (same `translation_group_id`), with its own slug and
/// publishing. Fields not given are copied from this entry. A page's translation goes under the
/// translation of its parent.
#[utoipa::path(
    post,
    path = "/entries/{id}/translations",
    operation_id = "create_content_translation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry to translate.")),
    request_body = TranslationRequest,
    responses(
        (status = 201, description = "The new translation (a draft).", body = Data<EntryResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry — or the translation of its parent page — is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "A translation in this language exists (code `content.translation_exists`), the parent page is not translated (code `content.parent_not_translated`), or the slug is taken (code `content.slug_taken`).", body = Problem),
        (status = 422, description = "The language is not supported or a value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_translation(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<TranslationRequest>,
) -> Result<Created<EntryResponse>, Error> {
    let entry =
        service.create_translation(&principal, id, &input.locale, input.fields.into()).await?;
    let location = format!("/content/entries/{}", entry.id);
    Ok(Created::new(location, tagged(&service, entry).await?.data))
}

/// Set the categories and tags of an entry.
///
/// Requires `content.write` (or `content.write_own` for an entry you created), and
/// `content.publish` (or `content.publish_own`) when the entry is published. Takes effect at
/// once, also for the published version.
#[utoipa::path(
    put,
    path = "/entries/{id}/terms",
    operation_id = "set_content_entry_terms",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    request_body = EntryTerms,
    responses(
        (status = 200, description = "The terms now set.", body = Data<EntryTerms>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (or `content.publish` / `content.publish_own` for a published entry; code `permission_denied`), or the entry is not yours and you only have the `_own` permission (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "An id is not a category or tag, or too many; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_terms(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<EntryTerms>,
) -> Result<Data<EntryTerms>, Error> {
    let term_ids = service.set_entry_terms(&principal, id, input.term_ids).await?;
    Ok(Data { data: EntryTerms { term_ids } })
}

/// Set the authors of an entry.
///
/// Requires `content.write` (or `content.write_own` for an entry you created), and
/// `content.publish` (or `content.publish_own`) when the entry is published. Takes effect at
/// once, also for the published version. With only `content.write_own` you may add only author
/// profiles linked to your own account (`user_id`); profiles already on the entry may stay.
#[utoipa::path(
    put,
    path = "/entries/{id}/authors",
    operation_id = "set_content_entry_authors",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    request_body = EntryAuthors,
    responses(
        (status = 200, description = "The authors now set.", body = Data<EntryAuthors>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (or `content.publish` / `content.publish_own` for a published entry; code `permission_denied`), or the entry is not yours and you only have the `_own` permission (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "An id is not an author, too many, or (with only `content.write_own`) a profile of someone else (`not_allowed`); see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_authors(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<EntryAuthors>,
) -> Result<Data<EntryAuthors>, Error> {
    let author_ids = service.set_entry_authors(&principal, id, input.author_ids).await?;
    Ok(Data { data: EntryAuthors { author_ids } })
}
