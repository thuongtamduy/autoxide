//! Endpoint đánh giá sau mua (D-201, D-202, D-203): người mua viết / sửa, danh sách và thống kê
//! công khai, phiếu "hữu ích", kiểm duyệt và trả lời của nhân viên, bản nháp tóm tắt AI.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::entity::{review, review_summary};
use crate::reviews::{PublicFilter, ReviewInput, StaffFilter};
use crate::service::OrdersService;

/// A review to write or edit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersReviewRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewRequest {
    /// 1 to 5 stars.
    #[schema(example = 5)]
    rating: i32,
    /// Up to 5000 characters; may be empty.
    #[serde(default)]
    body: String,
    /// Photos you uploaded (`POST /api/v1/media/me/uploads`), at most 6.
    #[serde(default)]
    media_ids: Vec<Uuid>,
}

impl From<ReviewRequest> for ReviewInput {
    fn from(request: ReviewRequest) -> Self {
        ReviewInput { rating: request.rating, body: request.body, media_ids: request.media_ids }
    }
}

/// A product review.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReview)]
pub(crate) struct ReviewResponse {
    id: Uuid,
    product_id: Uuid,
    variant_id: Uuid,
    rating: i32,
    body: String,
    media_ids: Vec<Uuid>,
    /// Always `true`: only buyers who received the item can review it.
    verified_buyer: bool,
    /// `published` or `hidden`.
    status: String,
    /// Code of the list `orders.review_hidden`, when hidden.
    hidden_reason: Option<String>,
    /// The shop's public answer.
    reply: Option<String>,
    replied_at: Option<Timestamp>,
    helpful_count: i32,
    /// Signed in: whether you voted "helpful" for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    voted_by_me: Option<bool>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

impl ReviewResponse {
    pub(crate) fn new(row: review::Model, voted: Option<bool>) -> Self {
        ReviewResponse {
            id: row.id,
            product_id: row.product_id,
            variant_id: row.variant_id,
            rating: row.rating,
            body: row.body,
            media_ids: serde_json::from_str(&row.media_ids).unwrap_or_default(),
            verified_buyer: true,
            status: row.status,
            hidden_reason: row.hidden_reason,
            reply: row.reply,
            replied_at: row.replied_at,
            helpful_count: row.helpful_count,
            voted_by_me: voted,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Review an item you received.
///
/// Signed in: a line of an order you placed that was delivered; one review per line. Published at
/// once (reviews are not held for approval).
#[utoipa::path(
    post,
    path = "/me/orders/{id}/lines/{line_id}/review",
    extensions(("x-permissions" = json!([]))),
    operation_id = "create_my_review",
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("line_id" = Uuid, Path, description = "Id of the order line."),
    ),
    request_body = ReviewRequest,
    responses(
        (status = 201, description = "The review.", body = Data<ReviewResponse>, headers(("Location" = String, description = "Path of the review."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "The item was not delivered to you (`orders.review_not_allowed`) or your account is not verified (`orders.account_not_verified`).", body = Problem),
        (status = 404, description = "No such order or line of yours; code `orders.order_not_found`.", body = Problem),
        (status = 409, description = "You already reviewed this line; code `orders.review_exists`.", body = Problem),
        (status = 422, description = "Invalid values, or photos not yours (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn create_review(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, line_id)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<ReviewRequest>,
) -> Result<Created<ReviewResponse>, Error> {
    let row =
        service.create_review(&tenant, principal.user_id(), id, line_id, input.into()).await?;
    let path = format!("/orders/public/reviews/{}", row.id);
    Ok(Created::new(path, ReviewResponse::new(row, None)))
}

/// Edit your review of an item.
///
/// Signed in: your review of this order line, within `AUTOXIDE_ORDERS_REVIEW_EDIT_DAYS` of writing
/// it.
#[utoipa::path(
    put,
    path = "/me/orders/{id}/lines/{line_id}/review",
    extensions(("x-permissions" = json!([]))),
    operation_id = "edit_my_review",
    params(
        ("id" = Uuid, Path, description = "Id of the order."),
        ("line_id" = Uuid, Path, description = "Id of the order line."),
    ),
    request_body = ReviewRequest,
    responses(
        (status = 200, description = "The review.", body = Data<ReviewResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No review of yours for this line; code `orders.review_not_found`.", body = Problem),
        (status = 409, description = "Too late to edit; code `orders.review_edit_closed`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn edit_review(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, line_id)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<ReviewRequest>,
) -> Result<Data<ReviewResponse>, Error> {
    let row = service.edit_review(&tenant, principal.user_id(), id, line_id, input.into()).await?;
    Ok(Data { data: ReviewResponse::new(row, None) })
}

/// Filters of the public review list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicReviewQuery {
    /// Only reviews with this many stars (1 to 5).
    rating: Option<i32>,
    /// Only reviews with (`true`) or without (`false`) photos.
    has_media: Option<bool>,
    /// `newest` (default) or `helpful` (most "helpful" votes first; the reader chooses it).
    sort: Option<String>,
}

/// List reviews of a product.
///
/// Public. Published reviews, newest first unless `sort=helpful`. Signed in, each review says
/// whether you voted for it.
#[utoipa::path(
    get,
    path = "/public/products/{product_id}/reviews",
    extensions(("x-public" = json!(true))),
    operation_id = "list_product_reviews",
    security((), ("bearer" = [])),
    params(("product_id" = Uuid, Path, description = "Id of the product."), PublicReviewQuery, PageQuery),
    responses(
        (status = 200, description = "A page of reviews.", body = Page<ReviewResponse>),
        (status = 400, description = "Bad cursor or limit.", body = Problem),
        (status = 422, description = "Invalid filter (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn public_reviews(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Option<Principal>,
    Path(product_id): Path<Uuid>,
    Query(query): Query<PublicReviewQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReviewResponse>, Error> {
    let mut problems = Vec::new();
    if query.rating.is_some_and(|rating| !(1..=5).contains(&rating)) {
        problems.push(FieldError::new("rating", "range", "must be 1 to 5"));
    }
    let helpful_first = match query.sort.as_deref() {
        None | Some("newest") => false,
        Some("helpful") => true,
        Some(_) => {
            problems.push(FieldError::new("sort", "invalid", "must be `newest` or `helpful`"));
            false
        }
    };
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let filter = PublicFilter { rating: query.rating, has_media: query.has_media, helpful_first };
    let page = PageRequest::try_from(page)?;
    let found = service.public_reviews(&tenant, product_id, &filter, &page).await?;
    let voted = match &principal {
        Some(principal) => {
            let ids: Vec<Uuid> = found.data.iter().map(|row| row.id).collect();
            Some(service.voted_by(&tenant, principal.user_id(), &ids).await?)
        }
        None => None,
    };
    let data = found
        .data
        .into_iter()
        .map(|row| {
            let mine = voted.as_ref().map(|voted| voted.contains(&row.id));
            ReviewResponse::new(row, mine)
        })
        .collect();
    Ok(Page { data, pagination: found.pagination })
}

/// Star counts and the approved AI summary of a product's reviews.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReviewStats)]
pub(crate) struct StatsResponse {
    count: i64,
    /// Average stars with one decimal (`"4.5"`); `null` without reviews.
    average: Option<String>,
    /// Number of reviews with 1, 2, 3, 4 and 5 stars.
    stars: [i64; 5],
    /// Reviews with photos.
    with_media: i64,
    /// Count and average stars per variant (size, colour…) that has reviews.
    by_variant: Vec<VariantStatsResponse>,
    summary: Option<SummaryResponse>,
}

/// Reviews of one variant.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReviewVariantStats)]
pub(crate) struct VariantStatsResponse {
    variant_id: Uuid,
    count: i64,
    /// Average stars with one decimal (`"4.5"`).
    average: String,
}

/// Trung bình một chữ số thập phân, làm tròn nửa lên, tính bằng số nguyên.
fn average(sum: i64, count: i64) -> Option<String> {
    (count > 0).then(|| {
        let tenths = (sum * 100 / count + 5) / 10;
        format!("{}.{}", tenths / 10, tenths % 10)
    })
}

/// A summary of reviews written with AI and approved by staff.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReviewSummary)]
pub(crate) struct SummaryResponse {
    id: Uuid,
    product_id: Uuid,
    locale: String,
    /// `pending`, `draft`, `approved`, `rejected`, `superseded` or `failed`.
    status: String,
    body: Option<String>,
    /// Themes buyers often mention.
    themes: Vec<String>,
    /// Always `true`: shown with an "AI-generated" label (D-109).
    ai_generated: bool,
    source_review_count: i32,
    generated_at: Option<Timestamp>,
    reviewed_at: Option<Timestamp>,
}

impl From<review_summary::Model> for SummaryResponse {
    fn from(row: review_summary::Model) -> Self {
        SummaryResponse {
            id: row.id,
            product_id: row.product_id,
            locale: row.locale,
            status: row.status,
            body: row.body,
            themes: serde_json::from_str(&row.themes).unwrap_or_default(),
            ai_generated: true,
            source_review_count: row.source_review_count,
            generated_at: row.generated_at,
            reviewed_at: row.reviewed_at,
        }
    }
}

/// Language of the summary.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct LocaleQuery {
    /// `vi` (default) or `en`.
    locale: Option<String>,
}

/// Star counts of a product's reviews.
///
/// Public. Counts of published reviews, the average, and the AI summary approved by staff (with
/// its AI label), if any.
#[utoipa::path(
    get,
    path = "/public/products/{product_id}/reviews/stats",
    extensions(("x-public" = json!(true))),
    operation_id = "get_product_review_stats",
    security(()),
    params(("product_id" = Uuid, Path, description = "Id of the product."), LocaleQuery),
    responses((status = 200, description = "The counts.", body = Data<StatsResponse>))
)]
pub(crate) async fn review_stats(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    Path(product_id): Path<Uuid>,
    Query(query): Query<LocaleQuery>,
) -> Result<Data<StatsResponse>, Error> {
    let locale = query.locale.unwrap_or_else(|| "vi".to_owned());
    let stats = service.review_stats(&tenant, product_id, &locale).await?;
    Ok(Data { data: stats_response(stats) })
}

/// Số sao như endpoint công khai trả (dùng chung với khối `orders.reviews`).
pub(crate) fn stats_response(stats: crate::review_stats::Stats) -> StatsResponse {
    let by_variant = stats
        .by_variant
        .iter()
        .map(|&(variant_id, count, sum)| VariantStatsResponse {
            variant_id,
            count,
            average: average(sum, count).unwrap_or_default(),
        })
        .collect();
    StatsResponse {
        count: stats.count,
        average: average(stats.rating_sum, stats.count),
        stars: stats.stars,
        with_media: stats.with_media,
        by_variant,
        summary: stats.summary.map(SummaryResponse::from),
    }
}

/// "Helpful" votes of a review.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReviewHelpful)]
pub(crate) struct HelpfulResponse {
    helpful_count: i32,
    voted_by_me: bool,
}

/// Vote a review "helpful".
///
/// Signed in. One vote per person; voting again changes nothing. Not for your own reviews, nor for
/// moderators of the shop selling the product. There is no "not helpful" vote.
#[utoipa::path(
    put,
    path = "/public/reviews/{id}/helpful",
    extensions(("x-permissions" = json!([]))),
    operation_id = "vote_review_helpful",
    params(("id" = Uuid, Path, description = "Id of the review.")),
    responses(
        (status = 200, description = "The votes.", body = Data<HelpfulResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "You moderate this shop's reviews; code `orders.review_vote_not_allowed`.", body = Problem),
        (status = 404, description = "No such published review; code `orders.review_not_found`.", body = Problem),
        (status = 409, description = "Your own review; code `orders.review_vote_own`.", body = Problem),
        (status = 429, description = "Too many votes; code `orders.review_votes_too_many`.", body = Problem),
    )
)]
pub(crate) async fn vote_helpful(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<HelpfulResponse>, Error> {
    let (helpful_count, voted_by_me) = service.vote_helpful(&tenant, &principal, id, true).await?;
    Ok(Data { data: HelpfulResponse { helpful_count, voted_by_me } })
}

/// Take back a "helpful" vote.
///
/// Signed in. Without a vote, nothing changes.
#[utoipa::path(
    delete,
    path = "/public/reviews/{id}/helpful",
    extensions(("x-permissions" = json!([]))),
    operation_id = "unvote_review_helpful",
    params(("id" = Uuid, Path, description = "Id of the review.")),
    responses(
        (status = 200, description = "The votes.", body = Data<HelpfulResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 404, description = "No such published review; code `orders.review_not_found`.", body = Problem),
    )
)]
pub(crate) async fn unvote_helpful(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<HelpfulResponse>, Error> {
    let (helpful_count, voted_by_me) = service.vote_helpful(&tenant, &principal, id, false).await?;
    Ok(Data { data: HelpfulResponse { helpful_count, voted_by_me } })
}

/// Filters of the staff review list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct StaffReviewQuery {
    /// `published` or `hidden`.
    status: Option<String>,
    /// Only reviews written after this time (e.g. the time you last looked).
    created_after: Option<Timestamp>,
    /// Only reviews of this product.
    product_id: Option<Uuid>,
}

impl StaffReviewQuery {
    fn filter(self) -> StaffFilter {
        StaffFilter {
            status: self.status,
            created_after: self.created_after,
            product_id: self.product_id,
        }
    }
}

/// List reviews to moderate.
///
/// Requires `orders.reviews.moderate` (reviews of shops in your scope), newest first.
#[utoipa::path(
    get,
    path = "/reviews",
    extensions(("x-permissions" = json!(["orders.reviews.moderate"]))),
    operation_id = "list_reviews",
    params(StaffReviewQuery, PageQuery),
    responses(
        (status = 200, description = "A page of reviews.", body = Page<ReviewResponse>),
        (status = 400, description = "Bad cursor or limit.", body = Problem),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reviews.moderate`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn staff_reviews(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<StaffReviewQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReviewResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    let found = service.staff_reviews(&tenant, &principal, &query.filter(), &page).await?;
    let data = found.data.into_iter().map(|row| ReviewResponse::new(row, None)).collect();
    Ok(Page { data, pagination: found.pagination })
}

/// A count of reviews.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = OrdersReviewCount)]
pub(crate) struct CountResponse {
    count: u64,
}

/// Count reviews to moderate.
///
/// Requires `orders.reviews.moderate`. With `created_after` set to the time you last looked
/// (`/api/v1/me/flags` key `orders.reviews.seen_at`), the number of new reviews.
#[utoipa::path(
    get,
    path = "/reviews/count",
    extensions(("x-permissions" = json!(["orders.reviews.moderate"]))),
    operation_id = "count_reviews",
    params(StaffReviewQuery),
    responses(
        (status = 200, description = "The count.", body = Data<CountResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reviews.moderate`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn count_reviews(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<StaffReviewQuery>,
) -> Result<Data<CountResponse>, Error> {
    let count = service.count_reviews(&tenant, &principal, &query.filter()).await?;
    Ok(Data { data: CountResponse { count } })
}

/// Hide or reopen a review.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersModerateReviewRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModerationRequest {
    /// `hide` or `reopen`.
    #[schema(example = "hide")]
    action: String,
    /// Required to hide: a code of the list `orders.review_hidden`.
    #[validate(length(min = 1, max = 64))]
    reason: Option<String>,
}

/// Hide or reopen a review.
///
/// Requires `orders.reviews.moderate` at the shop. A review is hidden only for a reason of the list
/// `orders.review_hidden` (Consumer Protection Law 19/2023: reviews are not blocked or hidden
/// otherwise); the author is told. Votes are kept and come back on reopening.
#[utoipa::path(
    post,
    path = "/reviews/{id}/moderation",
    extensions(("x-permissions" = json!(["orders.reviews.moderate"]))),
    operation_id = "moderate_review",
    params(("id" = Uuid, Path, description = "Id of the review.")),
    request_body = ModerationRequest,
    responses(
        (status = 200, description = "The review.", body = Data<ReviewResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reviews.moderate`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such review in your scope; code `orders.review_not_found`.", body = Problem),
        (status = 422, description = "Reason missing or not listed (`orders.moderation_reason_invalid`) or bad action (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn moderate(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ModerationRequest>,
) -> Result<Data<ReviewResponse>, Error> {
    // Không có quyền thì 403 trước khi xét `action`.
    principal.require(&crate::permissions::REVIEWS_MODERATE)?;
    let hide = match input.action.as_str() {
        "hide" => true,
        "reopen" => false,
        _ => {
            return Err(Error::validation(vec![FieldError::new(
                "action",
                "invalid",
                "must be `hide` or `reopen`",
            )]));
        }
    };
    let row = service.moderate_review(&tenant, &principal, id, hide, input.reason).await?;
    Ok(Data { data: ReviewResponse::new(row, None) })
}

/// The shop's answer.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersReplyReviewRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyRequest {
    #[validate(length(min = 1, max = 2000))]
    reply: String,
}

/// Answer a review publicly.
///
/// Requires `orders.reviews.moderate` at the shop. A new answer replaces the old one.
#[utoipa::path(
    post,
    path = "/reviews/{id}/reply",
    extensions(("x-permissions" = json!(["orders.reviews.moderate"]))),
    operation_id = "reply_review",
    params(("id" = Uuid, Path, description = "Id of the review.")),
    request_body = ReplyRequest,
    responses(
        (status = 200, description = "The review.", body = Data<ReviewResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reviews.moderate`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such review in your scope; code `orders.review_not_found`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
    )
)]
pub(crate) async fn reply(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReplyRequest>,
) -> Result<Data<ReviewResponse>, Error> {
    let row = service.reply_review(&tenant, &principal, id, &input.reply).await?;
    Ok(Data { data: ReviewResponse::new(row, None) })
}

/// Ask for an AI summary draft.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = OrdersSummaryRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct SummaryRequest {
    /// `vi` (default) or `en`.
    #[validate(length(min = 2, max = 8))]
    locale: Option<String>,
}

/// Write an AI summary draft of a product's reviews.
///
/// Requires `orders.reviews.moderate`. A background job asks the AI model for a draft
/// (`status: pending` → `draft`); it is shown publicly only after staff approve it, with an AI
/// label (D-109).
#[utoipa::path(
    post,
    path = "/products/{product_id}/review-summaries",
    extensions(("x-permissions" = json!(["orders.reviews.moderate"]))),
    operation_id = "request_review_summary",
    params(("product_id" = Uuid, Path, description = "Id of the product.")),
    request_body = SummaryRequest,
    responses(
        (status = 201, description = "The summary, `pending`.", body = Data<SummaryResponse>, headers(("Location" = String, description = "Path of the summary."))),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reviews.moderate`; code `permission_denied`.", body = Problem),
        (status = 422, description = "Invalid values (see `errors`).", body = Problem),
        (status = 503, description = "AI is not available; code `orders.ai_unavailable` or `ai.disabled`.", body = Problem),
    )
)]
pub(crate) async fn request_summary(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(product_id): Path<Uuid>,
    ValidJson(input): ValidJson<SummaryRequest>,
) -> Result<Created<SummaryResponse>, Error> {
    let locale = input.locale.unwrap_or_else(|| "vi".to_owned());
    let row = service.request_summary(&tenant, &principal, product_id, &locale).await?;
    let path = format!("/orders/review-summaries/{}", row.id);
    Ok(Created::new(path, row.into()))
}

/// Approve an AI summary draft.
///
/// Requires `orders.reviews.moderate`. It replaces the summary shown for the product and language.
#[utoipa::path(
    post,
    path = "/review-summaries/{id}/approve",
    extensions(("x-permissions" = json!(["orders.reviews.moderate"]))),
    operation_id = "approve_review_summary",
    params(("id" = Uuid, Path, description = "Id of the summary.")),
    responses(
        (status = 200, description = "The summary, `approved`.", body = Data<SummaryResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reviews.moderate`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such summary; code `orders.summary_not_found`.", body = Problem),
        (status = 409, description = "Not a draft; code `orders.summary_not_draft`.", body = Problem),
    )
)]
pub(crate) async fn approve_summary(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SummaryResponse>, Error> {
    let row = service.decide_summary(&tenant, &principal, id, true).await?;
    Ok(Data { data: row.into() })
}

/// Reject an AI summary draft.
///
/// Requires `orders.reviews.moderate`.
#[utoipa::path(
    post,
    path = "/review-summaries/{id}/reject",
    extensions(("x-permissions" = json!(["orders.reviews.moderate"]))),
    operation_id = "reject_review_summary",
    params(("id" = Uuid, Path, description = "Id of the summary.")),
    responses(
        (status = 200, description = "The summary, `rejected`.", body = Data<SummaryResponse>),
        (status = 401, description = "Not signed in; code `unauthenticated`.", body = Problem),
        (status = 403, description = "Missing `orders.reviews.moderate`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such summary; code `orders.summary_not_found`.", body = Problem),
        (status = 409, description = "Not a draft; code `orders.summary_not_draft`.", body = Problem),
    )
)]
pub(crate) async fn reject_summary(
    State(service): State<Arc<OrdersService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<SummaryResponse>, Error> {
    let row = service.decide_summary(&tenant, &principal, id, false).await?;
    Ok(Data { data: row.into() })
}
