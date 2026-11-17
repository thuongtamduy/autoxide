//! Endpoint REST của module `content`, dưới `/api/v1/content`. Handler chỉ lấy dữ liệu vào,
//! gọi service và trả kết quả; mọi logic (kể cả kiểm permission) nằm ở service. Mọi route
//! đăng ký qua `routes!` để có tài liệu OpenAPI (luật kiến trúc 6).

mod actions;
mod assist;
mod entries;
mod history;
mod library;
mod links;
mod menus;
mod public;
mod redirects;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_tenancy;
#[cfg(test)]
mod tests_web;

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::service::ContentService;

pub(crate) use assist::{SearchHit, SeoReport, SeoSuggestion, TranslationRequestResponse};
pub(crate) use entries::{EntriesQuery, EntryResponse, describe};
pub(crate) use history::{RevisionResponse, revision};
pub(crate) use public::{Resolution, resolution};

pub(crate) fn routes(service: Arc<ContentService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(entries::list_entries, entries::create_entry))
        .routes(routes!(entries::get_entry, entries::update_entry, actions::delete_entry))
        .routes(routes!(actions::publish_entry))
        .routes(routes!(actions::unpublish_entry))
        .routes(routes!(actions::archive_entry))
        .routes(routes!(actions::move_entry))
        .routes(routes!(actions::request_review, actions::withdraw_review))
        .routes(routes!(assist::seo_check))
        .routes(routes!(assist::seo_suggestions))
        .routes(routes!(assist::translate))
        .routes(routes!(assist::list_translation_requests))
        .routes(routes!(assist::get_translation_request))
        .routes(routes!(assist::search))
        .routes(routes!(assist::public_search))
        .routes(routes!(links::create_translation))
        .routes(routes!(links::set_terms))
        .routes(routes!(links::set_authors))
        .routes(routes!(library::list_terms, library::create_term))
        .routes(routes!(library::get_term, library::update_term, library::delete_term))
        .routes(routes!(library::list_authors, library::create_author))
        .routes(routes!(library::get_author, library::update_author, library::delete_author))
        .routes(routes!(menus::list_menus))
        .routes(routes!(menus::get_menu, menus::put_menu))
        .routes(routes!(history::list_revisions))
        .routes(routes!(history::get_revision))
        .routes(routes!(history::restore_revision))
        .routes(routes!(history::list_schedules, history::create_schedule))
        .routes(routes!(history::cancel_schedule))
        .routes(routes!(history::list_all_schedules))
        .routes(routes!(redirects::list_redirects, redirects::create_redirect))
        .routes(routes!(
            redirects::get_redirect,
            redirects::update_redirect,
            redirects::delete_redirect
        ))
        .routes(routes!(public::preview_token))
        .routes(routes!(public::resolve))
        .routes(routes!(public::list_posts))
        .routes(routes!(public::get_menu))
        .routes(routes!(public::list_terms))
        .routes(routes!(public::get_term))
        .routes(routes!(public::get_author))
        .routes(routes!(public::sitemap))
        .routes(routes!(public::feed))
        .routes(routes!(public::preview))
        .with_state(service)
}

/// A node of a body, in the ProseMirror / Tiptap JSON shape. The server accepts only these
/// `type`s (anything else is refused):
///
/// - blocks: `paragraph`, `heading` (`attrs.level` 2–4), `bulletList`, `orderedList`
///   (`attrs.start`), `listItem`, `blockquote`, `codeBlock` (`attrs.language`; plain
///   `text` only), `horizontalRule`, `image` (`attrs.mediaId` — a media item id — `alt`,
///   `caption`; no `src`), `table`, `tableRow`, `tableHeader`, `tableCell` (`attrs.colspan`,
///   `rowspan`, `colwidth`), `embed` (`attrs.url`, an `https://` address on an allowed host; or
///   `attrs.kind` + `attrs.id`, a record of another module such as `catalog.product`, which must
///   exist — public reads list it in `embeds`);
/// - inline: `text` (with `marks`), `hardBreak`.
///
/// The root is `{"type": "doc", "content": [...]}`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct BodyNode {
    /// Node type.
    #[serde(rename = "type")]
    kind: String,
    /// Attributes; only those listed for the type.
    attrs: Option<HashMap<String, Value>>,
    /// Child nodes.
    #[schema(no_recursion)]
    content: Option<Vec<BodyNode>>,
    /// Marks of a `text` node.
    marks: Option<Vec<BodyMark>>,
    /// The text of a `text` node.
    text: Option<String>,
}

/// A mark of a `text` node: `bold`, `italic`, `underline`, `strike`, `code`, or `link`
/// (`attrs.href`: `http://`, `https://`, `mailto:` or a path starting with `/`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct BodyMark {
    #[serde(rename = "type")]
    kind: String,
    attrs: Option<HashMap<String, Value>>,
}

/// Tham số `parent_id` của danh sách: `root` hay một UUID.
fn parent_filter(
    raw: Option<&str>,
    problems: &mut Vec<FieldError>,
) -> crate::entries::ParentFilter {
    use crate::entries::ParentFilter;
    match raw.map(str::trim) {
        None | Some("") => ParentFilter::Any,
        Some("root") => ParentFilter::Root,
        Some(raw) => Uuid::parse_str(raw).map(ParentFilter::Id).unwrap_or_else(|_| {
            problems.push(FieldError::new("parent_id", "invalid", "must be `root` or an entry id"));
            ParentFilter::Any
        }),
    }
}

/// Yêu cầu trang, gộp lỗi của nó với lỗi của bộ lọc để báo một lần.
pub(crate) fn page_request(
    query: PageQuery,
    mut problems: Vec<FieldError>,
) -> Result<PageRequest, Error> {
    let page = PageRequest::try_from(query);
    if let Err(error) = &page {
        problems.extend(error.fields().iter().cloned());
    }
    match page {
        Ok(page) if problems.is_empty() => Ok(page),
        _ => Err(Error::validation(problems)),
    }
}

/// Một giá trị liệt kê (`snake_case`) từ tham số query `field`; sai thì thêm lỗi.
pub(crate) fn choice<T: for<'de> Deserialize<'de>>(
    raw: Option<&str>,
    field: &str,
    allowed: &str,
    problems: &mut Vec<FieldError>,
) -> Option<T> {
    let raw = raw.map(str::trim).filter(|raw| !raw.is_empty())?;
    serde_json::from_value(Value::String(raw.to_owned()))
        .map_err(|_| {
            problems.push(
                FieldError::new(field, "invalid", "must be one of {allowed}")
                    .with_param("allowed", allowed),
            );
        })
        .ok()
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
