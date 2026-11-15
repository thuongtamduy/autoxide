//! Endpoint công khai (không cần đăng nhập; gửi token thì token phải đúng): chỉ đọc bản đã
//! đăng, không bao giờ trả bản nháp. Có `ETag` (→ 304) và `Cache-Control: public, no-cache`:
//! cache được nhưng hỏi lại máy chủ — website làm mới ngay nhờ webhook (D-102).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::BodyNode;
use crate::entries::Kind;
use crate::publishing::reading::PublicView;
use crate::publishing::redirects::Resolved;
use crate::service::ContentService;

/// An image, as readers load it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicMedia {
    /// Media item id.
    id: Uuid,
    /// Where to load it (versioned: cache it for long).
    url: String,
    width: Option<i32>,
    height: Option<i32>,
    /// Text alternative.
    alt: String,
    /// Smaller copies, narrowest first, for `srcset`.
    variants: Vec<PublicVariant>,
}

/// A smaller copy of an image.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicVariant {
    width: i32,
    height: i32,
    /// JPEG copy (PNG for images with transparency).
    url: String,
    /// The same copy as WebP, usually far smaller: offer it in a `<picture>`
    /// `<source type="image/webp">`. `null` while it is not made yet.
    webp_url: Option<String>,
}

fn media(item: Option<autoxide_media::api::MediaRef>, alt: &str) -> Option<PublicMedia> {
    item.map(|item| PublicMedia {
        alt: if alt.is_empty() { item.alt } else { alt.to_owned() },
        id: item.id,
        url: item.url,
        width: item.width,
        height: item.height,
        variants: item
            .variants
            .into_iter()
            .map(|copy| PublicVariant {
                width: copy.width,
                height: copy.height,
                url: copy.url,
                webp_url: copy.webp_url,
            })
            .collect(),
    })
}

/// Search engine and sharing data.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicSeo {
    /// Title for search engines; falls back to the entry title when empty.
    meta_title: String,
    meta_description: String,
    /// Image for social sharing; `null` when none (use `featured_media`).
    og_image: Option<PublicMedia>,
    /// Canonical address; empty when the entry's own address is canonical.
    canonical_url: String,
    noindex: bool,
}

/// Another language version of the same entry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicTranslation {
    #[schema(example = "en")]
    locale: String,
    path: String,
}

/// A category or tag of an entry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicTerm {
    id: Uuid,
    /// `category` or `tag`.
    taxonomy: String,
    name: String,
    slug: String,
    /// The language of `name` and `slug` (the default language when not translated).
    locale: String,
}

/// An author of an entry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicAuthor {
    id: Uuid,
    display_name: String,
    slug: String,
    avatar_media_id: Option<Uuid>,
}

/// A published post or page, as readers see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicEntryResponse {
    /// Stable id (also the id of feed items).
    id: Uuid,
    kind: Kind,
    locale: String,
    translation_group_id: Uuid,
    parent_id: Option<Uuid>,
    /// Address computed by the server, without domain or language prefix.
    #[schema(example = "/gioi-thieu/doi-ngu")]
    path: String,
    title: String,
    slug: String,
    excerpt: String,
    #[schema(value_type = BodyNode)]
    body: Value,
    body_schema_version: i32,
    word_count: i32,
    reading_time_minutes: i32,
    featured_media: Option<PublicMedia>,
    /// The images used in `body` (nodes of type `image`), in order of appearance: find one by
    /// the `mediaId` of the node to get its size and smaller copies for `srcset`. An image
    /// that is no longer public is left out. `alt` here is the one of the media library; the
    /// node carries its own.
    body_media: Vec<PublicMedia>,
    seo: PublicSeo,
    /// The date shown as the publication date.
    published_at: DateTime<Utc>,
    first_published_at: DateTime<Utc>,
    /// Last change of the published content (sitemap `lastmod`, feed `updated`).
    content_modified_at: DateTime<Utc>,
    /// Other published languages of this entry, for `hreflang`.
    translations: Vec<PublicTranslation>,
    /// Categories and tags; the first category is the main one.
    terms: Vec<PublicTerm>,
    /// Authors; the first is the main one.
    authors: Vec<PublicAuthor>,
    /// The content was generated by AI. Show readers a notice when your rules (for example
    /// Article 50 of the EU AI Act) ask for one.
    ai_generated: bool,
    /// A person reviewed the AI-generated content and took editorial responsibility when it
    /// was published. Always `false` when `ai_generated` is `false`.
    ai_reviewed: bool,
}

pub(crate) fn public_entry(view: PublicView) -> PublicEntryResponse {
    let PublicView { row, featured, og, body_media, translations, terms, authors } = view;
    PublicEntryResponse {
        featured_media: media(featured, &row.featured_media_alt),
        body_media: body_media.into_iter().filter_map(|item| media(Some(item), "")).collect(),
        seo: PublicSeo {
            og_image: media(og, ""),
            meta_title: row.meta_title,
            meta_description: row.meta_description,
            canonical_url: row.canonical_url,
            noindex: row.noindex,
        },
        translations: translations
            .into_iter()
            .map(|(locale, path)| PublicTranslation { locale, path })
            .collect(),
        terms: terms
            .into_iter()
            .map(|term| PublicTerm {
                id: term.id,
                taxonomy: term.taxonomy,
                name: term.name,
                slug: term.slug,
                locale: term.locale,
            })
            .collect(),
        authors: authors
            .into_iter()
            .map(|author| PublicAuthor {
                id: author.id,
                display_name: author.display_name,
                slug: author.slug,
                avatar_media_id: author.avatar_media_id,
            })
            .collect(),
        kind: Kind::parse(&row.kind).unwrap_or(Kind::Post),
        body: serde_json::from_str(&row.body).unwrap_or(Value::Null),
        id: row.entry_id,
        locale: row.locale,
        translation_group_id: row.translation_group_id,
        parent_id: row.parent_id,
        path: row.path,
        title: row.title,
        slug: row.slug,
        excerpt: row.excerpt,
        body_schema_version: row.body_schema_version,
        word_count: row.word_count,
        reading_time_minutes: row.reading_time_minutes,
        published_at: row.published_at,
        first_published_at: row.first_published_at,
        content_modified_at: row.content_modified_at,
        // Từ thời điểm xác nhận, không từ người xác nhận: xoá tài khoản bỏ trống
        // `ai_reviewed_by` nhưng sự thật "đã có người duyệt" vẫn đúng.
        ai_reviewed: row.ai_generated && row.ai_reviewed_at.is_some(),
        ai_generated: row.ai_generated,
    }
}

/// What lives at an address.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Resolution {
    /// A published entry: render it.
    Entry { entry: Box<PublicEntryResponse> },
    /// The address moved: answer with `status` and a `Location` of `location` (an address
    /// path in this language, or a full URL).
    Redirect { status: i32, location: String },
    /// Taken down for good: answer 410.
    Gone,
}

/// The address to look up.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ResolveQuery {
    /// Language of the address.
    #[param(example = "vi")]
    locale: String,
    /// The address, without domain or language prefix.
    #[param(example = "/gioi-thieu/doi-ngu")]
    path: String,
}

/// `Cache-Control` của mọi endpoint công khai: cache được, nhưng hỏi lại máy chủ (rẻ nhờ 304).
pub(crate) fn public_cache(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, no-cache"));
    response
}

/// Điều có ở đường dẫn `path`, kèm phiên bản (khi là một mục). Dùng chung cho REST và MCP.
pub(crate) async fn resolution(
    service: &ContentService,
    locale: &str,
    path: &str,
) -> Result<(Resolution, Option<DateTime<Utc>>), Error> {
    Ok(match service.resolve(locale, path).await? {
        Resolved::Entry(row) => {
            let version = row.updated_at;
            let view = service.public_views(vec![*row]).await?.into_iter().next();
            let view =
                view.ok_or_else(|| Error::internal("public view of a published entry is missing"))?;
            (Resolution::Entry { entry: Box::new(public_entry(view)) }, Some(version))
        }
        Resolved::Redirect { status, location } => {
            (Resolution::Redirect { status, location }, None)
        }
        Resolved::Gone => (Resolution::Gone, None),
    })
}

/// Look up an address (public).
///
/// No sign-in needed. Tells the website, in one call, what to do with a request for an
/// address in a language: render a published entry, redirect, or answer 410. Drafts are
/// never returned, and a missing translation is 404 (no fallback to another language).
#[utoipa::path(
    get,
    path = "/public/resolve",
    extensions(("x-public" = json!(true))),
    operation_id = "resolve_content_path",
    security((), ("bearer" = [])),
    params(ResolveQuery),
    responses(
        (status = 200, description = "What lives at the address: `type` is `entry`, `redirect` or `gone`.", body = Data<Resolution>, headers(("ETag" = String, description = "Fingerprint of the whole answer (for an entry, including its terms, authors and translations); send it in `If-None-Match` to get 304."))),
        (status = 404, description = "Nothing at this address; code `content.not_found`.", body = Problem),
        (status = 422, description = "The language is not supported or the path is malformed; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn resolve(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Query(query): Query<ResolveQuery>,
) -> Result<Response, Error> {
    let (resolution, version) = resolution(&service, &query.locale, &query.path).await?;
    let data = Data { data: resolution };
    // ETag là dấu vân tay của CẢ câu trả lời, không phải `updated_at` của bản công khai: câu
    // trả lời còn gộp danh mục, thẻ, tác giả, bản dịch, ảnh — đổi những thứ đó không chạm
    // `updated_at`, và một ETag theo thời gian sẽ làm website nhận 304 với nội dung cũ.
    let etag = version.map(|_| fingerprint(&data)).transpose()?;
    let mut response = data.into_response();
    if let Some(etag) = etag
        && let Ok(value) = HeaderValue::from_str(&etag)
    {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(public_cache(response))
}

/// `ETag` mạnh từ nội dung: SHA-256 của JSON, 16 byte đầu, base64url, trong ngoặc kép.
fn fingerprint<T: Serialize>(value: &T) -> Result<String, Error> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value)
        .map_err(|error| Error::internal(format!("cannot serialize the answer: {error}")))?;
    let digest = Sha256::digest(&bytes);
    let short = digest.get(..16).unwrap_or_default();
    Ok(format!("\"{}\"", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(short)))
}

use autoxide_kernel::pagination::{Page, PageQuery};
use axum::extract::Path;

use super::page_request;
use crate::publishing::reading::PostFilter;
use crate::taxonomy::terms::Taxonomy;

/// Ghép thông tin cho một trang bản công khai.
async fn views(
    service: &ContentService,
    page: Page<crate::publishing::Published>,
) -> Result<Page<PublicEntryResponse>, Error> {
    let Page { data, pagination } = page;
    let data = service.public_views(data).await?.into_iter().map(public_entry).collect();
    Ok(Page { data, pagination })
}

/// Which published posts to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PostsQuery {
    /// Language of the posts.
    #[param(example = "vi")]
    locale: String,
    /// Only posts with this category or tag (its id; find it by slug with
    /// `GET /api/v1/content/public/terms/{taxonomy}/{slug}`).
    term_id: Option<Uuid>,
    /// Only posts by this author.
    author_id: Option<Uuid>,
    /// Only posts whose title, summary or body contains this text, ignoring case and accents.
    /// At most 200 characters.
    q: Option<String>,
}

/// List published posts (public).
///
/// No sign-in needed. Newest first by publication date; follow `next_cursor` for more.
#[utoipa::path(
    get,
    path = "/public/posts",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_content_posts",
    security((), ("bearer" = [])),
    params(PostsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of published posts.", body = Page<PublicEntryResponse>),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_posts(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Query(query): Query<PostsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Response, Error> {
    let mut problems = Vec::new();
    let locale = service.check_locale(Some(&query.locale), "locale", &mut problems);
    // Câu tìm dài vô hạn thành một câu `LIKE` đắt trên mọi bài, cho bất kỳ ai gọi.
    let max = crate::ai::semantic::QUERY_MAX;
    if query.q.as_deref().is_some_and(|q| q.trim().chars().count() > max) {
        problems.push(autoxide_kernel::error::FieldError::new(
            "q",
            "length",
            format!("must be at most {max} characters"),
        ));
    }
    let page = page_request(paging, problems)?;
    let filter =
        PostFilter { locale, term_id: query.term_id, author_id: query.author_id, search: query.q };
    let page = views(&service, service.public_posts(&filter, &page).await?).await?;
    Ok(public_cache(page.into_response()))
}

/// A navigation menu as readers see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicMenu {
    key: String,
    /// The language actually returned (the default language when the menu is not translated).
    locale: String,
    items: Vec<PublicMenuItem>,
}

/// An item of a public menu.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicMenuItem {
    label: String,
    /// The address: a path in the site (for an entry), or the address given. Empty for a
    /// term: build its address from `term`.
    href: String,
    entry_id: Option<Uuid>,
    term: Option<PublicTerm>,
    #[schema(no_recursion)]
    children: Vec<PublicMenuItem>,
}

fn menu_items(nodes: Vec<crate::taxonomy::menus::PublicNode>) -> Vec<PublicMenuItem> {
    nodes
        .into_iter()
        .map(|node| PublicMenuItem {
            label: node.label,
            href: node.href,
            entry_id: node.entry_id,
            term: node.term.map(|term| PublicTerm {
                id: term.id,
                taxonomy: term.taxonomy,
                name: term.name,
                slug: term.slug,
                locale: term.locale,
            }),
            children: menu_items(node.children),
        })
        .collect()
}

/// The language of a public read.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct LocaleQuery {
    #[param(example = "vi")]
    locale: String,
}

/// Get a navigation menu (public).
///
/// No sign-in needed. Items pointing to entries that are not published are left out, with
/// their sub-items. A menu missing in the language falls back to the default language.
#[utoipa::path(
    get,
    path = "/public/menus/{key}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_content_menu",
    security((), ("bearer" = [])),
    params(("key" = String, Path, description = "Key of the menu, e.g. `main`."), LocaleQuery),
    responses(
        (status = 200, description = "The menu.", body = Data<PublicMenu>),
        (status = 404, description = "No such menu; code `content.menu_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_menu(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Path(key): Path<String>,
    Query(query): Query<LocaleQuery>,
) -> Result<Response, Error> {
    let (menu, nodes) = service.public_menu(&key, &query.locale).await?;
    let data = PublicMenu { key: menu.key, locale: menu.locale, items: menu_items(nodes) };
    Ok(public_cache(Data { data }.into_response()))
}

/// A category or tag as readers see it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicTermPage {
    #[serde(flatten)]
    term: PublicTerm,
    description: String,
    meta_title: String,
    meta_description: String,
    /// Parent category.
    parent_id: Option<Uuid>,
    /// The slug in every language, for `hreflang`.
    translations: Vec<PublicTranslation>,
}

/// Get a category or tag by slug (public).
///
/// No sign-in needed. Looks in the language, then in the default language (`locale` says
/// which was used).
#[utoipa::path(
    get,
    path = "/public/terms/{taxonomy}/{slug}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_content_term",
    security((), ("bearer" = [])),
    params(("taxonomy" = Taxonomy, Path, description = "`category` or `tag`."), ("slug" = String, Path, description = "Slug of the term."), LocaleQuery),
    responses(
        (status = 200, description = "The term.", body = Data<PublicTermPage>),
        (status = 404, description = "No such term; code `content.term_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn get_term(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Path((taxonomy, slug)): Path<(Taxonomy, String)>,
    Query(query): Query<LocaleQuery>,
) -> Result<Response, Error> {
    let (full, shown) = service.term_by_slug(taxonomy, &query.locale, &slug).await?;
    let text = full.texts.iter().find(|text| text.locale == shown.locale).cloned();
    let text = text.ok_or_else(|| Error::internal("term name missing"))?;
    let data = PublicTermPage {
        description: text.description,
        meta_title: text.meta_title,
        meta_description: text.meta_description,
        parent_id: full.term.parent_id,
        translations: full
            .texts
            .iter()
            .map(|text| PublicTranslation { locale: text.locale.clone(), path: text.slug.clone() })
            .collect(),
        term: PublicTerm {
            id: shown.id,
            taxonomy: shown.taxonomy,
            name: shown.name,
            slug: shown.slug,
            locale: shown.locale,
        },
    };
    Ok(public_cache(Data { data }.into_response()))
}

/// An author as readers see them.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PublicAuthorPage {
    id: Uuid,
    display_name: String,
    slug: String,
    bio: String,
    avatar: Option<PublicMedia>,
    website_url: String,
}

/// Get an author by slug (public).
///
/// No sign-in needed. The account behind the profile is not shown.
#[utoipa::path(
    get,
    path = "/public/authors/{slug}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_content_author",
    security((), ("bearer" = [])),
    params(("slug" = String, Path, description = "Slug of the author.")),
    responses(
        (status = 200, description = "The author.", body = Data<PublicAuthorPage>),
        (status = 404, description = "No such author; code `content.author_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_author(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Path(slug): Path<String>,
) -> Result<Response, Error> {
    let author = service.author_by_slug(&slug).await?;
    let avatar = match author.avatar_media_id {
        Some(id) => service
            .media
            .find(id)
            .await?
            .filter(|item| item.visibility == autoxide_media::api::MediaVisibility::Public),
        None => None,
    };
    let data = PublicAuthorPage {
        avatar: media(avatar, ""),
        id: author.id,
        display_name: author.display_name,
        slug: author.slug,
        bio: author.bio,
        website_url: author.website_url,
    };
    Ok(public_cache(Data { data }.into_response()))
}

/// One address of the sitemap.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SitemapItem {
    id: Uuid,
    kind: Kind,
    locale: String,
    path: String,
    /// For `lastmod`.
    content_modified_at: DateTime<Utc>,
    /// Leave it out of the sitemap when `true`.
    noindex: bool,
    /// The other languages of the same entry, for `hreflang` alternates.
    translations: Vec<PublicTranslation>,
}

/// Which addresses to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SitemapQuery {
    /// Only this language; omitted: every language.
    locale: Option<String>,
}

/// List every published address, for a sitemap (public).
///
/// No sign-in needed. Follow `next_cursor` (up to 200 per page) to get them all.
#[utoipa::path(
    get,
    path = "/public/sitemap",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_content_sitemap",
    security((), ("bearer" = [])),
    params(SitemapQuery, PageQuery),
    responses(
        (status = 200, description = "One page of addresses.", body = Page<SitemapItem>),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn sitemap(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Query(query): Query<SitemapQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Response, Error> {
    let mut problems = Vec::new();
    let locale = query
        .locale
        .as_deref()
        .map(|locale| service.check_locale(Some(locale), "locale", &mut problems));
    let page = page_request(paging, problems)?;
    let found = service.sitemap(locale.as_deref(), &page).await?;
    let mut translations = service.translations_of(&found.data).await?;
    let page = found.map(|row| SitemapItem {
        translations: translations
            .remove(&row.entry_id)
            .unwrap_or_default()
            .into_iter()
            .map(|(locale, path)| PublicTranslation { locale, path })
            .collect(),
        kind: Kind::parse(&row.kind).unwrap_or(Kind::Post),
        id: row.entry_id,
        locale: row.locale,
        path: row.path,
        content_modified_at: row.content_modified_at,
        noindex: row.noindex,
    });
    Ok(public_cache(page.into_response()))
}

/// Which feed to build.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FeedQuery {
    #[param(example = "vi")]
    locale: String,
    /// How many posts, 1 to 100. Defaults to 20.
    #[param(minimum = 1, maximum = 100)]
    limit: Option<u32>,
}

/// The newest posts of a language, for RSS, Atom or JSON Feed (public).
///
/// No sign-in needed. Each post has its body, summary, authors and images; use its `id` (a
/// UUID that never changes) as the feed item id.
#[utoipa::path(
    get,
    path = "/public/feed",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_content_feed",
    security((), ("bearer" = [])),
    params(FeedQuery),
    responses(
        (status = 200, description = "The newest posts, newest first.", body = Data<Vec<PublicEntryResponse>>),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn feed(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Query(query): Query<FeedQuery>,
) -> Result<Response, Error> {
    let mut problems = Vec::new();
    let locale = service.check_locale(Some(&query.locale), "locale", &mut problems);
    let limit = query.limit.unwrap_or(20);
    if !(1..=100).contains(&limit) {
        problems.push(autoxide_kernel::error::FieldError::new(
            "limit",
            "range",
            "must be 1 to 100",
        ));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let rows = service.feed(&locale, limit).await?;
    let data: Vec<PublicEntryResponse> =
        service.public_views(rows).await?.into_iter().map(public_entry).collect();
    Ok(public_cache(Data { data }.into_response()))
}

/// A preview link.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PreviewQuery {
    /// The token from `POST /api/v1/content/entries/{id}/preview-token`.
    token: String,
}

/// The draft of a post or page, as a preview link shows it: the content the website renders,
/// without the internal fields of the admin view (who created it, who asked for a review, how it
/// was created).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PreviewEntryResponse {
    id: Uuid,
    kind: Kind,
    #[schema(example = "vi")]
    locale: String,
    translation_group_id: Uuid,
    /// The parent page (pages only); `null` at the root.
    parent_id: Option<Uuid>,
    status: crate::entries::Status,
    title: String,
    slug: String,
    excerpt: String,
    #[schema(value_type = BodyNode)]
    body: Value,
    body_schema_version: i32,
    word_count: i32,
    reading_time_minutes: i32,
    /// Main image (a media item id).
    featured_media_id: Option<Uuid>,
    featured_media_alt: String,
    meta_title: String,
    meta_description: String,
    /// Image for social sharing (a media item id).
    og_media_id: Option<Uuid>,
    canonical_url: String,
    noindex: bool,
    /// The content was generated by AI.
    ai_generated: bool,
    /// Last change of the draft.
    updated_at: DateTime<Utc>,
    /// Categories and tags, in order (the first category is the main one).
    term_ids: Vec<Uuid>,
    /// Authors, in order (the first is the main one).
    author_ids: Vec<Uuid>,
}

/// Bản nháp như link xem trước trả: chỉ nội dung, không ID người dùng hay trường nội bộ (ai tạo,
/// ai xin duyệt, tạo bằng gì) — token xem trước có thể tới tay người ngoài.
async fn preview_entry(
    service: &ContentService,
    entry: crate::entries::Entry,
) -> Result<PreviewEntryResponse, Error> {
    let ids = [entry.id];
    let term_ids = service.term_ids(&ids).await?.remove(&entry.id);
    let author_ids = service.author_ids(&ids).await?.remove(&entry.id);
    Ok(PreviewEntryResponse {
        kind: Kind::parse(&entry.kind).unwrap_or(Kind::Post),
        status: crate::entries::Status::of(&entry),
        body: serde_json::from_str(&entry.body).unwrap_or(Value::Null),
        term_ids: term_ids.unwrap_or_default(),
        author_ids: author_ids.unwrap_or_default(),
        id: entry.id,
        locale: entry.locale,
        translation_group_id: entry.translation_group_id,
        parent_id: entry.parent_id,
        title: entry.title,
        slug: entry.slug,
        excerpt: entry.excerpt,
        body_schema_version: entry.body_schema_version,
        word_count: entry.word_count,
        reading_time_minutes: entry.reading_time_minutes,
        featured_media_id: entry.featured_media_id,
        featured_media_alt: entry.featured_media_alt,
        meta_title: entry.meta_title,
        meta_description: entry.meta_description,
        og_media_id: entry.og_media_id,
        canonical_url: entry.canonical_url,
        noindex: entry.noindex,
        ai_generated: entry.ai_generated,
        updated_at: entry.updated_at,
    })
}

/// Read a draft through a preview link (public).
///
/// No sign-in needed: the signed token is the permission. Returns the content of the latest
/// draft, without the internal fields of the admin view. Answers with `Cache-Control: no-store`.
#[utoipa::path(
    get,
    path = "/public/preview",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_content_preview",
    security((), ("bearer" = [])),
    params(PreviewQuery),
    responses(
        (status = 200, description = "The draft.", body = Data<PreviewEntryResponse>),
        (status = 404, description = "The link is invalid or expired; code `content.preview_token_invalid`.", body = Problem),
        (status = 503, description = "Previews are off on this server; code `content.preview_disabled`.", body = Problem),
    )
)]
pub(crate) async fn preview(
    State(service): State<Arc<ContentService>>,
    _principal: Option<Principal>,
    Query(query): Query<PreviewQuery>,
) -> Result<Response, Error> {
    let entry = service.preview(&query.token).await?;
    let data = preview_entry(&service, entry).await?;
    let mut response = Data { data }.into_response();
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// A preview link for a draft.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct PreviewToken {
    /// Give it to `GET /api/v1/content/public/preview?token=…`.
    token: String,
    expires_at: DateTime<Utc>,
}

/// Make a preview link for the draft of an entry.
///
/// Requires `content.write`, or `content.write_own` for an entry you created. Anyone with the
/// token can read the draft until it expires (10 minutes by default).
#[utoipa::path(
    post,
    path = "/entries/{id}/preview-token",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "create_content_preview_token",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 200, description = "The token.", body = Data<PreviewToken>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 503, description = "Previews are off on this server; code `content.preview_disabled`.", body = Problem),
    )
)]
pub(crate) async fn preview_token(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<PreviewToken>, Error> {
    let (token, expires_at) = service.preview_token(&principal, id).await?;
    Ok(Data { data: PreviewToken { token, expires_at } })
}

#[cfg(test)]
mod tests {
    use autoxide_media::api::{MediaRef, MediaVariant, MediaVisibility};

    use super::*;

    /// Bản công khai mang cả địa chỉ JPEG lẫn WebP của từng cỡ (D-196); `alt` của bài thay
    /// `alt` của thư viện media khi có.
    #[test]
    fn anh_cong_khai_co_ban_webp() {
        let id = Uuid::now_v7();
        let item = MediaRef {
            id,
            visibility: MediaVisibility::Public,
            content_type: "image/png".to_owned(),
            width: Some(1200),
            height: Some(800),
            alt: "library alt".to_owned(),
            url: "/u?v=1".to_owned(),
            variants: vec![
                MediaVariant {
                    width: 320,
                    height: 213,
                    url: "/u?v=1&w=320".to_owned(),
                    webp_url: Some("/u?v=1&w=320&format=webp".to_owned()),
                },
                MediaVariant {
                    width: 640,
                    height: 427,
                    url: "/u?v=1&w=640".to_owned(),
                    webp_url: None,
                },
            ],
        };
        let public = serde_json::to_value(media(Some(item), "entry alt")).unwrap();
        assert_eq!(public["alt"], "entry alt");
        assert_eq!(
            public["variants"],
            serde_json::json!([
                {"width": 320, "height": 213, "url": "/u?v=1&w=320", "webp_url": "/u?v=1&w=320&format=webp"},
                {"width": 640, "height": 427, "url": "/u?v=1&w=640", "webp_url": null},
            ])
        );
        assert!(media(None, "x").is_none());
    }
}
