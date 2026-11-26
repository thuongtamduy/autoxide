//! Bộ nháp chiến dịch từ một chương trình khuyến mại (bước 67): yêu cầu (202, chạy nền) và kết quả.
//! Logic ở `ai/campaign.rs`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use autoxide_spike_macros::Schema as ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::ai::campaign::{CampaignDraft, CampaignRequest, ChannelResult};
use crate::service::ContentService;

/// A set of campaign drafts made from one promotion, and the outcome of each channel.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CampaignDraftsResponse {
    id: Uuid,
    promotion_id: Uuid,
    /// Website of the landing page and the banner; `null`: the requester's own node.
    org_id: Option<Uuid>,
    locale: String,
    /// The channels asked for: `page`, `banner`, `newsletter`.
    channels: Vec<String>,
    /// Extra wishes of the requester (may be empty).
    instruction: String,
    banner_placement: Option<String>,
    newsletter_list_id: Option<Uuid>,
    /// `queued`, `succeeded` (every channel has a draft), `partial` or `failed`.
    status: String,
    /// Calls made to the model for the banner and mail copy (a refused answer is retried at most
    /// twice).
    tries: i32,
    /// Why the whole set failed, e.g. `promotions.not_found` when the promotion is gone.
    error_code: Option<String>,
    /// One result per channel once finished.
    results: Vec<ChannelResultResponse>,
    requested_by: Uuid,
    #[schema(value_type = String, format = DateTime)]
    created_at: Timestamp,
    #[schema(value_type = Option<String>, format = DateTime)]
    finished_at: Option<Timestamp>,
}

/// The draft made for one channel. Nothing is published or sent: review each draft in its module.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ChannelResultResponse {
    /// `page`, `banner` or `newsletter`.
    channel: String,
    /// `created` or `failed`.
    status: String,
    /// `content.entry` (a draft page), `content.banner` (a draft banner) or `newsletter.campaign` (a
    /// draft campaign).
    kind: Option<String>,
    /// Id of the draft.
    id: Option<Uuid>,
    /// `page` only: the AI turn building the layout; follow it with `GET /entries/{id}/ai/thread`.
    turn_id: Option<Uuid>,
    /// Why this channel failed, e.g. `content.campaign_copy_invalid`, `permission_denied`.
    error_code: Option<String>,
}

pub(crate) fn response(row: CampaignDraft) -> CampaignDraftsResponse {
    let results: Vec<ChannelResult> =
        row.results.as_deref().and_then(|text| serde_json::from_str(text).ok()).unwrap_or_default();
    CampaignDraftsResponse {
        id: row.id,
        promotion_id: row.promotion_id,
        org_id: row.org_id,
        locale: row.locale,
        channels: serde_json::from_str(&row.channels).unwrap_or_default(),
        instruction: row.instruction,
        banner_placement: row.banner_placement,
        newsletter_list_id: row.newsletter_list_id,
        status: row.status,
        tries: row.tries,
        error_code: row.error_code,
        results: results
            .into_iter()
            .map(|result| ChannelResultResponse {
                channel: result.channel,
                status: result.status,
                kind: result.kind,
                id: result.id,
                turn_id: result.turn_id,
                error_code: result.error_code,
            })
            .collect(),
        requested_by: row.requested_by,
        created_at: row.created_at,
        finished_at: row.finished_at,
    }
}

/// A request for campaign drafts.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CampaignDraftsRequest {
    /// The promotion (module `promotions`) to draft the campaign for.
    pub promotion_id: Uuid,
    /// Website (organization node) of the landing page and the banner. Omit for your own node.
    pub org_id: Option<Uuid>,
    /// Language of the drafts; one of the site's languages. Omit for the default language.
    pub locale: Option<String>,
    /// What to draft: `page` (a landing page built from blocks), `banner` (a text banner linking to
    /// the promotion), `newsletter` (a campaign of the `newsletter` module).
    pub channels: Vec<String>,
    /// Extra wishes in plain words (at most 2000 characters), e.g. "playful tone, mention free
    /// shipping".
    pub instruction: Option<String>,
    /// `banner` channel: key of the banner placement (`GET /placements`).
    pub banner_placement: Option<String>,
    /// `newsletter` channel: the list the campaign is written for.
    pub newsletter_list_id: Option<Uuid>,
}

impl From<CampaignDraftsRequest> for CampaignRequest {
    fn from(input: CampaignDraftsRequest) -> Self {
        CampaignRequest {
            promotion_id: input.promotion_id,
            org_id: input.org_id,
            locale: input.locale,
            channels: input.channels,
            instruction: input.instruction,
            banner_placement: input.banner_placement,
            newsletter_list_id: input.newsletter_list_id,
        }
    }
}

/// Draft a campaign from a promotion.
///
/// Requires `content.write` (or `content.write_own`), the plan feature `content.ai_suggestions`, and
/// reading the promotion (`promotions.read`); the `banner` channel also needs `content.banners`, the
/// `newsletter` channel `newsletter.manage`. Runs in the background: follow it with
/// `GET /campaign-drafts/{id}`. From the promotion's real data the AI drafts, per channel: a landing page
/// (blocks `promotions.flash_sale` / `promotions.voucher_claim`), a text banner linking to the
/// promotion, and an advertising newsletter campaign. Every result is a **draft** in its own module —
/// nothing is published or sent; numbers in the copy must come from the promotion. Counts once toward
/// the plan limit `content.ai_page_generations`.
#[utoipa::path(
    post,
    path = "/campaign-drafts",
    extensions(("x-permissions" = json!(["content.write", "content.write_own"]))),
    operation_id = "create_content_campaign_drafts",
    security(("bearer" = [])),
    request_body = CampaignDraftsRequest,
    responses(
        (status = 202, description = "Queued.", body = Data<CampaignDraftsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission (`permission_denied`), or the plan lacks `content.ai_suggestions`.", body = Problem),
        (status = 404, description = "No promotion has this id in your scope (code `promotions.not_found`), or `org_id` is not a unit of this tenant (code `org_not_found`).", body = Problem),
        (status = 422, description = "A channel is unknown or misses its `banner_placement` / `newsletter_list_id`, the language or the instruction is invalid; see `errors`.", body = Problem),
        (status = 429, description = "The plan's monthly AI page builds are used up; code `quota_exhausted`.", body = Problem),
        (status = 503, description = "AI is off on this server (`ai.disabled`), or the `promotions` / `newsletter` module is not installed (`content.campaign_module_unavailable`).", body = Problem),
    )
)]
pub(crate) async fn create_campaign_drafts(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CampaignDraftsRequest>,
) -> Result<(StatusCode, Data<CampaignDraftsResponse>), Error> {
    let row = service.request_campaign_drafts(&tenant, &principal, input.into()).await?;
    Ok((StatusCode::ACCEPTED, Data { data: response(row) }))
}

/// Get a set of campaign drafts.
///
/// Requires `content.read`; someone else's request also needs `content.write`. Shows the status
/// and, once finished, the draft made for each channel.
#[utoipa::path(
    get,
    path = "/campaign-drafts/{id}",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_campaign_drafts",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    responses(
        (status = 200, description = "The request and its results.", body = Data<CampaignDraftsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No request has this id, or it is someone else's and you lack `content.write`; code `content.campaign_draft_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_campaign_drafts(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CampaignDraftsResponse>, Error> {
    Ok(Data { data: response(service.campaign_drafts(&tenant, &principal, id).await?) })
}
