//! Endpoint trợ giúp biên tập (D-109): kiểm SEO (không AI), gợi ý SEO (AI). Không endpoint nào
//! ở đây ghi vào mục; người xem gợi ý rồi áp bằng `PATCH /entries/{id}`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::seo::{Finding, Level, Suggestion};
use crate::service::ContentService;
use crate::translate::TranslationRequest;

/// How serious a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FindingLevel {
    /// Fix it before publishing (for example an image without a text alternative).
    Error,
    /// Worth a look.
    Warning,
}

/// Something to fix or review.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SeoFinding {
    /// Stable code for machines, e.g. `seo.meta_title_too_long`, `seo.image_alt_missing`,
    /// `seo.heading_skipped`, `seo.title_duplicate`, `seo.translation_missing`.
    #[schema(example = "seo.image_alt_missing")]
    code: String,
    level: FindingLevel,
    /// The field, or the place in the body JSON (e.g. `body.content[2].attrs.alt`).
    #[schema(example = "body.content[2].attrs.alt")]
    field: String,
    /// What to fix, in English.
    message: String,
}

impl From<Finding> for SeoFinding {
    fn from(finding: Finding) -> Self {
        SeoFinding {
            code: finding.code.to_owned(),
            level: match finding.level {
                Level::Error => FindingLevel::Error,
                Level::Warning => FindingLevel::Warning,
            },
            field: finding.field,
            message: finding.message,
        }
    }
}

/// The SEO findings of a draft.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SeoReport {
    /// Empty when there is nothing to fix.
    findings: Vec<SeoFinding>,
}

impl From<Vec<Finding>> for SeoReport {
    fn from(findings: Vec<Finding>) -> Self {
        SeoReport { findings: findings.into_iter().map(SeoFinding::from).collect() }
    }
}

/// Suggestions of the AI model; nothing was changed.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SeoSuggestion {
    /// Title for search engines (aims at 60 characters or fewer).
    meta_title: String,
    /// Description for search engines (aims at 120 to 160 characters).
    meta_description: String,
    /// A short summary.
    excerpt: String,
    /// Up to 5 existing categories or tags of the entry's language that fit; never new ones.
    term_ids: Vec<Uuid>,
    /// The model that answered.
    model: String,
}

impl From<Suggestion> for SeoSuggestion {
    fn from(suggestion: Suggestion) -> Self {
        SeoSuggestion {
            meta_title: suggestion.meta_title,
            meta_description: suggestion.meta_description,
            excerpt: suggestion.excerpt,
            term_ids: suggestion.term_ids,
            model: suggestion.model,
        }
    }
}

/// Check the SEO of a draft.
///
/// Requires `content.read`. Works without AI: missing or too long title and description for
/// search engines, missing summary or sharing image, images without a text alternative (in the
/// body and the main image), skipped heading levels, the same title as another entry of the
/// language, `noindex` on a published entry, missing translations. Changes nothing.
#[utoipa::path(
    get,
    path = "/entries/{id}/seo-check",
    operation_id = "check_content_seo",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 200, description = "The findings; an empty list when there is nothing to fix.", body = Data<SeoReport>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
    )
)]
pub(crate) async fn seo_check(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SeoReport>, Error> {
    Ok(Data { data: SeoReport::from(service.seo_check(&principal, id).await?) })
}

/// Suggest SEO fields with AI.
///
/// Requires `content.write`, or `content.write_own` for an entry you created. The AI model
/// reads the draft (title, summary, text) and the categories and tags of its language, and
/// suggests a title and description for search engines, a summary and up to 5 existing terms.
/// Nothing is saved: show the suggestions, then apply the ones a person accepts with
/// `PATCH /entries/{id}` (and `PUT /entries/{id}/terms`). Uses AI tokens; see
/// `GET /api/v1/ai/status`.
#[utoipa::path(
    post,
    path = "/entries/{id}/seo-suggestions",
    operation_id = "suggest_content_seo",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry.")),
    responses(
        (status = 200, description = "The suggestions; nothing was changed.", body = Data<SeoSuggestion>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 422, description = "The AI model declined the content (code `ai.refused`) or its answer was cut off (code `ai.output_truncated`).", body = Problem),
        (status = 429, description = "Today's AI token limit is used up; code `ai.quota_exceeded`, with `Retry-After`.", body = Problem),
        (status = 503, description = "AI is off (code `ai.disabled`), misconfigured (code `ai.misconfigured`), the provider is busy (code `ai.provider_unavailable`, maybe with `Retry-After`), or its answer had the wrong shape twice (code `ai.output_invalid`).", body = Problem),
    )
)]
pub(crate) async fn seo_suggestions(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SeoSuggestion>, Error> {
    Ok(Data { data: service.suggest_seo(&principal, id).await?.into() })
}

/// The language to translate into.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TranslateRequest {
    /// One of the languages of `AUTOXIDE_CONTENT_LOCALES`, other than the entry's.
    #[schema(example = "vi")]
    pub locale: String,
}

/// Where an AI translation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TranslationStatus {
    /// Waiting for a worker.
    Queued,
    /// Being translated.
    Running,
    /// Done: `result_entry_id` is the translation's draft.
    Succeeded,
    /// Not done: see `error_code`.
    Failed,
}

/// An AI translation request.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct TranslationRequestResponse {
    id: Uuid,
    /// The entry being translated.
    source_entry_id: Uuid,
    /// The revision of the source that is translated (its content when the request was made).
    source_revision_id: Uuid,
    #[schema(example = "vi")]
    target_locale: String,
    /// The translation that existed when the request was made and will be updated; `null`: a new
    /// translation is created.
    target_entry_id: Option<Uuid>,
    status: TranslationStatus,
    /// Why it failed, e.g. `content.translation_conflict` (someone changed the translation
    /// meanwhile; it was not overwritten), `content.translation_invalid`, `ai.refused`,
    /// `ai.output_truncated`, `ai.misconfigured`, `content.translation_canceled`.
    error_code: Option<String>,
    /// The translation's draft, once `succeeded`. It is never published automatically and is
    /// marked `ai_generated`.
    result_entry_id: Option<Uuid>,
    created_by: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<TranslationRequest> for TranslationRequestResponse {
    fn from(request: TranslationRequest) -> Self {
        TranslationRequestResponse {
            status: match request.status.as_str() {
                "running" => TranslationStatus::Running,
                "succeeded" => TranslationStatus::Succeeded,
                "failed" => TranslationStatus::Failed,
                _ => TranslationStatus::Queued,
            },
            id: request.id,
            source_entry_id: request.source_entry_id,
            source_revision_id: request.source_revision_id,
            target_locale: request.target_locale,
            target_entry_id: request.target_entry_id,
            error_code: request.error_code,
            result_entry_id: request.result_entry_id,
            created_by: request.created_by,
            created_at: request.created_at,
            updated_at: request.updated_at,
        }
    }
}

/// `202 Accepted` + `Location` của yêu cầu dịch.
pub(crate) struct Accepted(TranslationRequestResponse);

impl IntoResponse for Accepted {
    fn into_response(self) -> Response {
        let location = format!("/api/v1/content/translation-requests/{}", self.0.id);
        let mut response = (StatusCode::ACCEPTED, Data { data: self.0 }).into_response();
        if let Ok(value) = HeaderValue::from_str(&location) {
            response.headers_mut().insert(header::LOCATION, value);
        }
        response
    }
}

/// Translate an entry with AI.
///
/// Requires `content.write`, or `content.write_own` when you created the entry (and the
/// existing translation, if any). Answers at once with a request (`202`, `Location`); a
/// background job translates the entry as it is now — title, summary, SEO fields, image alt
/// texts and the body, keeping its structure, links and images — into the draft of the
/// translation in `locale`: a new translation, or the existing one if nobody changes it
/// meanwhile (otherwise the request fails with `content.translation_conflict` and nothing is
/// overwritten). The translation is never published automatically and is marked
/// `ai_generated`. Follow the request with `GET /api/v1/content/translation-requests/{id}`, or
/// subscribe a webhook to `content.translation_succeeded` / `content.translation_failed`.
/// Retry safely with an `Idempotency-Key` header. Uses AI tokens.
#[utoipa::path(
    post,
    path = "/entries/{id}/translate",
    operation_id = "translate_content_entry",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the entry to translate.")),
    request_body = TranslateRequest,
    responses(
        (status = 202, description = "Queued; `Location` is the request.", body = Data<TranslationRequestResponse>, headers(("Location" = String, description = "Address of the translation request."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` or `content.write_own` (code `permission_denied`), or the entry (or its translation) is not yours and you only have `content.write_own` (code `content.not_owner`).", body = Problem),
        (status = 404, description = "No entry has this id; code `content.not_found`.", body = Problem),
        (status = 409, description = "A translation into this language is already queued or running (code `content.translation_in_progress`), or the parent page has no translation yet (code `content.parent_not_translated`).", body = Problem),
        (status = 422, description = "The language is not supported or is the entry's own; see `errors`.", body = Problem),
        (status = 503, description = "AI is off on this server; code `ai.disabled`.", body = Problem),
    )
)]
pub(crate) async fn translate(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<TranslateRequest>,
) -> Result<Accepted, Error> {
    let request = service.request_translation(&principal, id, &input.locale).await?;
    Ok(Accepted(request.into()))
}

/// Get an AI translation request.
///
/// Requires `content.read`, or being the one who made the request.
#[utoipa::path(
    get,
    path = "/translation-requests/{id}",
    operation_id = "get_content_translation_request",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the translation request.")),
    responses(
        (status = 200, description = "The request.", body = Data<TranslationRequestResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read` (or `content.write` / `content.write_own`); code `permission_denied`.", body = Problem),
        (status = 404, description = "No translation request you may see has this id; code `content.translation_request_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_translation_request(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<TranslationRequestResponse>, Error> {
    Ok(Data { data: service.translation_request(&principal, id).await?.into() })
}

/// List the AI translation requests of an entry.
///
/// Requires `content.read`. The requests in which this entry is the source, newest first.
#[utoipa::path(
    get,
    path = "/entries/{id}/translation-requests",
    operation_id = "list_content_translation_requests",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the source entry."), PageQuery),
    responses(
        (status = 200, description = "One page of requests.", body = Page<TranslationRequestResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_translation_requests(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Result<Page<TranslationRequestResponse>, Error> {
    let page = PageRequest::try_from(query)?;
    let found = service.translation_requests_of(&principal, id, &page).await?;
    Ok(found.map(TranslationRequestResponse::from))
}

/// What to search for.
#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SearchParams {
    /// Words or a question, 1 to 200 characters. Matches by meaning (when embeddings are on) and
    /// by words, ignoring case and accents.
    #[param(example = "đi đâu ở Đà Lạt")]
    pub q: String,
    /// Language to search in; omitted: the default language.
    #[param(example = "vi")]
    pub locale: Option<String>,
    /// `post` or `page`; omitted: both.
    #[param(value_type = Option<crate::entries::Kind>, inline)]
    pub kind: Option<String>,
    /// Results to return, 1 to 50. Defaults to 10.
    pub limit: Option<u32>,
}

impl SearchParams {
    fn query(self) -> Result<crate::semantic::SearchQuery, Error> {
        let mut problems = Vec::new();
        let kind = super::choice(self.kind.as_deref(), "kind", "`post`, `page`", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        Ok(crate::semantic::SearchQuery { q: self.q, locale: self.locale, kind, limit: self.limit })
    }
}

/// How a result matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MatchedBy {
    /// Close in meaning (embeddings).
    Meaning,
    /// Contains the words.
    Text,
}

/// A search result.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SearchHit {
    entry_id: Uuid,
    kind: crate::entries::Kind,
    locale: String,
    title: String,
    /// Admin search: the entry's status. Public search: absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<crate::entries::Status>,
    /// Public search: the address of the published entry. Admin search: absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    /// Combined rank score (Reciprocal Rank Fusion of meaning and words); compare only within
    /// one search.
    score: f32,
    /// Similarity in meaning (cosine, -1 to 1) of the best matching passage; `null` when only
    /// the words matched.
    similarity: Option<f32>,
    /// The best matching passage, or the summary, up to 300 characters.
    snippet: String,
    /// `meaning`, `text` or both. Only `text` when embeddings are off.
    matched_by: Vec<MatchedBy>,
}

impl From<crate::semantic::Hit> for SearchHit {
    fn from(hit: crate::semantic::Hit) -> Self {
        SearchHit {
            kind: crate::entries::Kind::parse(&hit.kind).unwrap_or(crate::entries::Kind::Post),
            status: hit.status.as_deref().map(|status| match status {
                "published" => crate::entries::Status::Published,
                "archived" => crate::entries::Status::Archived,
                _ => crate::entries::Status::Draft,
            }),
            matched_by: hit
                .matched_by
                .iter()
                .map(|how| if *how == "meaning" { MatchedBy::Meaning } else { MatchedBy::Text })
                .collect(),
            entry_id: hit.entry_id,
            locale: hit.locale,
            title: hit.title,
            path: hit.path,
            score: hit.score,
            similarity: hit.similarity,
            snippet: hit.snippet,
        }
    }
}

/// Kết quả tìm dưới dạng `Page` (top-k: không có trang kế), cùng envelope với mọi danh sách.
fn page_of(hits: Vec<crate::semantic::Hit>, limit: Option<u32>) -> Page<SearchHit> {
    Page {
        data: hits.into_iter().map(SearchHit::from).collect(),
        pagination: autoxide_kernel::pagination::Pagination::new(limit.unwrap_or(10), None),
    }
}

/// Search posts and pages by meaning and words.
///
/// Requires `content.read`. Searches the drafts of one language: by meaning (when the server has
/// embeddings on) and by words (ignoring case and accents), combined into one ranking. Each
/// result tells how it matched and gives the best passage. The best `limit` results come in one
/// page (`next_cursor` is always `null`).
#[utoipa::path(
    get,
    path = "/search",
    operation_id = "search_content",
    security(("bearer" = [])),
    params(SearchParams),
    responses(
        (status = 200, description = "The best results, best first.", body = Page<SearchHit>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn search(
    State(service): State<Arc<ContentService>>,
    principal: Principal,
    Query(params): Query<SearchParams>,
) -> Result<Page<SearchHit>, Error> {
    let limit = params.limit;
    let hits = service.search(Some(&principal), &params.query()?, false).await?;
    Ok(page_of(hits, limit))
}

/// Search published posts and pages by meaning and words (public).
///
/// No sign-in needed; off unless the server sets `AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH=true`
/// (then `404` with code `content.semantic_search_disabled`). Like `GET /search`, but only
/// published entries, each with its `path`. Searches by meaning have a budget per minute
/// (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_PER_MINUTE`); past it, results match words only.
#[utoipa::path(
    get,
    path = "/public/search",
    operation_id = "search_public_content",
    security((), ("bearer" = [])),
    params(SearchParams),
    responses(
        (status = 200, description = "The best results, best first.", body = Page<SearchHit>),
        (status = 404, description = "Public search is off on this server; code `content.semantic_search_disabled`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn public_search(
    State(service): State<Arc<ContentService>>,
    principal: Option<Principal>,
    Query(params): Query<SearchParams>,
) -> Result<Page<SearchHit>, Error> {
    let limit = params.limit;
    let hits = service.search(principal.as_ref(), &params.query()?, true).await?;
    Ok(page_of(hits, limit))
}
