//! Endpoint chiến dịch (`/campaigns…`). Đọc cần `newsletter.read`; soạn, sửa, xoá cần
//! `newsletter.manage`; gửi, hẹn giờ, tạm dừng, tiếp tục, huỷ, gửi thử cần `newsletter.send`.

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

use super::{choice, page_request, present, uuid_param};
use crate::campaigns::{
    Action, Campaign, CampaignFilter, CampaignInput, CampaignStatus, DeliveryCounts,
};
use crate::service::NewsletterService;
use crate::text::Locale;

/// A campaign: one mail sent to the confirmed subscribers of a list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct CampaignResponse {
    pub id: Uuid,
    pub list_id: Uuid,
    pub subject: String,
    /// Short text shown after the subject in some mail apps (HTML mails only).
    pub preheader: Option<String>,
    /// Plain-text body (every mail has it).
    pub body_text: String,
    /// HTML body, if any; stored and sent as is, NOT sanitised. An admin interface must
    /// show it only inside a sandboxed `<iframe sandbox>` (no scripts, no same origin), never
    /// inserted into its own page.
    pub body_html: Option<String>,
    /// Language of the footer added to every mail.
    pub locale: Locale,
    /// Advertising: the subject gets `AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL` and each address
    /// gets at most `AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY` such mails a day.
    pub advertising: bool,
    pub status: CampaignStatus,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Where an automatic campaign came from (`content.entry` for a new post).
    pub source: Option<String>,
    pub source_ref: Option<String>,
    /// Progress by delivery status.
    pub progress: DeliveryCounts,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Campaign> for CampaignResponse {
    fn from(campaign: Campaign) -> Self {
        let row = campaign.row;
        CampaignResponse {
            status: CampaignStatus::stored(&row.status),
            locale: Locale::stored(&row.locale),
            id: row.id,
            list_id: row.list_id,
            subject: row.subject,
            preheader: row.preheader,
            body_text: row.body_text,
            body_html: row.body_html,
            advertising: row.advertising,
            scheduled_at: row.scheduled_at,
            started_at: row.started_at,
            finished_at: row.finished_at,
            source: row.source,
            source_ref: row.source_ref,
            progress: campaign.counts,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A new draft campaign.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCampaignRequest {
    /// The list to send to.
    list_id: Uuid,
    /// 1 to 200 characters, one line.
    subject: String,
    /// Up to 200 characters.
    preheader: Option<String>,
    /// Plain-text body, up to 200000 characters. A footer with the unsubscribe link and the
    /// sender's address is added to every mail.
    body_text: String,
    /// HTML body, up to 500000 bytes, sent as is (the footer is added). Not sanitised: admin
    /// interfaces must preview it only inside a sandboxed `<iframe sandbox>`.
    body_html: Option<String>,
    /// Language of the footer; omitted: `en`.
    locale: Option<Locale>,
    /// Omitted: `false`.
    advertising: Option<bool>,
}

/// Changes to a draft or scheduled campaign. Omitted fields stay as they are.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateCampaignRequest {
    list_id: Option<Uuid>,
    subject: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    preheader: Option<Option<String>>,
    body_text: Option<String>,
    /// Not sanitised: admin interfaces must preview it only inside a sandboxed
    /// `<iframe sandbox>`. `null` removes it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    body_html: Option<Option<String>>,
    locale: Option<Locale>,
    advertising: Option<bool>,
}

impl From<UpdateCampaignRequest> for CampaignInput {
    fn from(input: UpdateCampaignRequest) -> Self {
        CampaignInput {
            list_id: input.list_id,
            subject: input.subject,
            preheader: input.preheader,
            body_text: input.body_text,
            body_html: input.body_html,
            locale: input.locale,
            advertising: input.advertising,
        }
    }
}

/// Which campaigns to list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CampaignsQuery {
    #[param(value_type = Option<CampaignStatus>, inline)]
    pub status: Option<String>,
    #[param(value_type = Option<Uuid>)]
    pub list_id: Option<String>,
}

/// When to start sending.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = NewsletterScheduleRequest)]
pub(crate) struct ScheduleRequest {
    /// A time in the future (at most 366 days ahead), with a time zone.
    at: DateTime<Utc>,
}

/// Where to send a test mail.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestRequest {
    #[schema(example = "editor@example.com")]
    email: String,
}

/// List campaigns.
///
/// Requires `newsletter.read`. Newest first, with progress; follow `pagination.next_cursor`
/// with the same filters for more.
#[utoipa::path(
    get,
    path = "/campaigns",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "list_newsletter_campaigns",
    security(("bearer" = [])),
    params(CampaignsQuery, PageQuery),
    responses(
        (status = 200, description = "One page of campaigns.", body = Page<CampaignResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_campaigns(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Query(query): Query<CampaignsQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<CampaignResponse>, Error> {
    let mut problems = Vec::new();
    let filter = CampaignFilter {
        status: choice(
            query.status.as_deref(),
            "status",
            "`draft`, `scheduled`, `sending`, `paused`, `sent`, `cancelled`",
            &mut problems,
        ),
        list_id: uuid_param(query.list_id.as_deref(), "list_id", &mut problems),
    };
    let page = page_request(paging, problems)?;
    Ok(service.list_campaigns(&principal, &filter, &page).await?.map(Into::into))
}

/// Create a draft campaign.
///
/// Requires `newsletter.manage`. Nothing is sent until `send` or `schedule` (which need
/// `newsletter.send`). Every problem is reported at once in `errors`. Retry safely with an
/// `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/campaigns",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "create_newsletter_campaign",
    security(("bearer" = [])),
    request_body = CreateCampaignRequest,
    responses(
        (status = 201, description = "Created as a draft.", body = Data<CampaignResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    ValidJson(input): ValidJson<CreateCampaignRequest>,
) -> Result<Created<CampaignResponse>, Error> {
    let input = CampaignInput {
        list_id: Some(input.list_id),
        subject: Some(input.subject),
        preheader: input.preheader.map(Some),
        body_text: Some(input.body_text),
        body_html: input.body_html.map(Some),
        locale: input.locale,
        advertising: input.advertising,
    };
    let campaign = service.create_campaign(&principal, input).await?;
    Ok(Created::new(format!("/newsletter/campaigns/{}", campaign.row.id), campaign.into()))
}

/// Get a campaign, with its progress.
///
/// Requires `newsletter.read`.
#[utoipa::path(
    get,
    path = "/campaigns/{id}",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "get_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign.")),
    responses(
        (status = 200, description = "The campaign.", body = Data<CampaignResponse>, headers(("ETag" = String, description = "Version; send it in `If-Match` to change, send or schedule the campaign."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign has this id; code `newsletter.campaign_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<CampaignResponse>, Error> {
    let campaign = service.get_campaign(&principal, id).await?;
    Ok(Tagged { version: campaign.row.updated_at, data: campaign.into() })
}

/// A campaign mail exactly as subscribers get it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct CampaignPreviewResponse {
    /// The subject, with the advertising label when the campaign is advertising.
    subject: String,
    /// The plain-text body with the footer (list name, preferences link, postal address).
    body_text: String,
    /// The HTML body with the preheader and the footer; `null` for a text-only campaign. Show it
    /// only inside a sandboxed `<iframe sandbox>`, like `body_html`.
    body_html: Option<String>,
}

/// Preview a campaign mail.
///
/// Requires `newsletter.read`. The mail as it will be sent — subject label, footer, HTML
/// wrapping — with a placeholder token in its links (they do nothing). Needs the same settings
/// as sending; see `GET /api/v1/newsletter/settings`.
#[utoipa::path(
    get,
    path = "/campaigns/{id}/preview",
    extensions(("x-permissions" = json!(["newsletter.read"]))),
    operation_id = "preview_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign.")),
    responses(
        (status = 200, description = "The mail.", body = Data<CampaignPreviewResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign has this id; code `newsletter.campaign_not_found`.", body = Problem),
        (status = 503, description = "A setting campaign mails need is missing: code `public_url_missing`, `newsletter.preferences_url_missing`, or `newsletter.postal_address_missing` (advertising campaigns); the message names the variable to set.", body = Problem),
    )
)]
pub(crate) async fn preview_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CampaignPreviewResponse>, Error> {
    let preview = service.preview(&principal, id).await?;
    Ok(Data {
        data: CampaignPreviewResponse {
            subject: preview.subject,
            body_text: preview.text,
            body_html: preview.html,
        },
    })
}

/// Change a campaign.
///
/// Requires `newsletter.manage`. `If-Match` is required. Only a draft or scheduled campaign
/// can be changed. Changing a scheduled campaign moves it back to `draft` and drops its
/// schedule: someone with `newsletter.send` must schedule or send it again, so nobody can
/// change a reviewed campaign behind their back. A request that changes nothing keeps it
/// scheduled.
#[utoipa::path(
    patch,
    path = "/campaigns/{id}",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "update_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateCampaignRequest,
    responses(
        (status = 200, description = "The campaign after the change.", body = Data<CampaignResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign has this id; code `newsletter.campaign_not_found`.", body = Problem),
        (status = 409, description = "The campaign is no longer a draft or scheduled; code `newsletter.campaign_not_editable`.", body = Problem),
        (status = 412, description = "The campaign changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateCampaignRequest>,
) -> Result<Tagged<CampaignResponse>, Error> {
    let expected = if_match.required()?;
    let campaign = service.update_campaign(&principal, id, input.into(), expected).await?;
    Ok(Tagged { version: campaign.row.updated_at, data: campaign.into() })
}

/// Delete a campaign.
///
/// Requires `newsletter.manage`. `If-Match` is required. Only a campaign that never sent (a
/// draft, a scheduled one, or one cancelled before sending) can be deleted.
#[utoipa::path(
    delete,
    path = "/campaigns/{id}",
    extensions(("x-permissions" = json!(["newsletter.manage"]))),
    operation_id = "delete_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign has this id; code `newsletter.campaign_not_found`.", body = Problem),
        (status = 409, description = "The campaign already sent mail; code `newsletter.campaign_not_deletable`.", body = Problem),
        (status = 412, description = "The campaign changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn delete_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<StatusCode, Error> {
    let expected = if_match.required()?;
    service.delete_campaign(&principal, id, expected).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Send a campaign now.
///
/// Requires `newsletter.send`. `If-Match` is required, so that exactly the version you read
/// is sent. Takes every confirmed subscriber of the list at that moment, then hands the mails
/// to the mail queue in batches at `AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE`. Suppressed,
/// blocked and unsubscribed people are skipped. Needs `AUTOXIDE_HTTP_PUBLIC_URL` and
/// `AUTOXIDE_NEWSLETTER_PREFERENCES_URL` (and `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS` for
/// advertising). Follow the progress with `GET /campaigns/{id}`.
#[utoipa::path(
    post,
    path = "/campaigns/{id}/send",
    extensions(("x-permissions" = json!(["newsletter.send"]))),
    operation_id = "send_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    responses(
        (status = 200, description = "Sending started.", body = Data<CampaignResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.send`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign (or no list) has this id; code `newsletter.campaign_not_found` or `newsletter.list_not_found`.", body = Problem),
        (status = 409, description = "The campaign is not a draft or scheduled; code `newsletter.campaign_invalid_transition`.", body = Problem),
        (status = 412, description = "The campaign changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
        (status = 503, description = "A setting needed to send is missing; codes `public_url_missing`, `newsletter.preferences_url_missing`, `newsletter.postal_address_missing`.", body = Problem),
    )
)]
pub(crate) async fn send_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<CampaignResponse>, Error> {
    let expected = if_match.required()?;
    let campaign = service.change_state(&principal, id, Action::Send, expected).await?;
    Ok(Tagged { version: campaign.row.updated_at, data: campaign.into() })
}

/// Schedule a campaign.
///
/// Requires `newsletter.send`. `If-Match` is required. The campaign starts sending at `at`
/// (scheduling again moves the time). It can still be changed, sent at once or cancelled
/// until then. Needs the same settings as sending.
#[utoipa::path(
    post,
    path = "/campaigns/{id}/schedule",
    extensions(("x-permissions" = json!(["newsletter.send"]))),
    operation_id = "schedule_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = ScheduleRequest,
    responses(
        (status = 200, description = "Scheduled.", body = Data<CampaignResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.send`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign (or no list) has this id; code `newsletter.campaign_not_found` or `newsletter.list_not_found`.", body = Problem),
        (status = 409, description = "The campaign is not a draft or scheduled; code `newsletter.campaign_invalid_transition`.", body = Problem),
        (status = 412, description = "The campaign changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "`at` is not in the future; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
        (status = 503, description = "A setting needed to send is missing; codes `public_url_missing`, `newsletter.preferences_url_missing`, `newsletter.postal_address_missing`.", body = Problem),
    )
)]
pub(crate) async fn schedule_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<ScheduleRequest>,
) -> Result<Tagged<CampaignResponse>, Error> {
    let expected = if_match.required()?;
    let campaign =
        service.change_state(&principal, id, Action::Schedule(input.at), expected).await?;
    Ok(Tagged { version: campaign.row.updated_at, data: campaign.into() })
}

/// Pause a campaign.
///
/// Requires `newsletter.send`. Stops after the batch being sent; `resume` sends to the
/// recipients left. `If-Match` is checked when sent.
#[utoipa::path(
    post,
    path = "/campaigns/{id}/pause",
    extensions(("x-permissions" = json!(["newsletter.send"]))),
    operation_id = "pause_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign."), ("If-Match" = String, Header, description = "Optional: the `ETag` you read.")),
    responses(
        (status = 200, description = "Paused.", body = Data<CampaignResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.send`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign has this id; code `newsletter.campaign_not_found`.", body = Problem),
        (status = 409, description = "The campaign is not sending; code `newsletter.campaign_invalid_transition`.", body = Problem),
        (status = 412, description = "The campaign changed since you read it; code `precondition_failed`.", body = Problem),
    )
)]
pub(crate) async fn pause_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<CampaignResponse>, Error> {
    let expected = if_match.expected()?;
    let campaign = service.change_state(&principal, id, Action::Pause, expected).await?;
    Ok(Tagged { version: campaign.row.updated_at, data: campaign.into() })
}

/// Resume a paused campaign.
///
/// Requires `newsletter.send`. Sends to the recipients not handled yet — nobody gets the mail
/// twice. `If-Match` is checked when sent.
#[utoipa::path(
    post,
    path = "/campaigns/{id}/resume",
    extensions(("x-permissions" = json!(["newsletter.send"]))),
    operation_id = "resume_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign."), ("If-Match" = String, Header, description = "Optional: the `ETag` you read.")),
    responses(
        (status = 200, description = "Sending again.", body = Data<CampaignResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.send`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign has this id; code `newsletter.campaign_not_found`.", body = Problem),
        (status = 409, description = "The campaign is not paused; code `newsletter.campaign_invalid_transition`.", body = Problem),
        (status = 412, description = "The campaign changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 503, description = "A setting needed to send is missing; codes `public_url_missing`, `newsletter.preferences_url_missing`, `newsletter.postal_address_missing`.", body = Problem),
    )
)]
pub(crate) async fn resume_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<CampaignResponse>, Error> {
    let expected = if_match.expected()?;
    let campaign = service.change_state(&principal, id, Action::Resume, expected).await?;
    Ok(Tagged { version: campaign.row.updated_at, data: campaign.into() })
}

/// Cancel a campaign.
///
/// Requires `newsletter.send`. Stops a scheduled, sending or paused campaign for good; mails
/// not sent yet are skipped. `If-Match` is checked when sent.
#[utoipa::path(
    post,
    path = "/campaigns/{id}/cancel",
    extensions(("x-permissions" = json!(["newsletter.send"]))),
    operation_id = "cancel_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign."), ("If-Match" = String, Header, description = "Optional: the `ETag` you read.")),
    responses(
        (status = 200, description = "Cancelled.", body = Data<CampaignResponse>, headers(("ETag" = String, description = "New version."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.send`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign has this id; code `newsletter.campaign_not_found`.", body = Problem),
        (status = 409, description = "The campaign is a draft or already finished; code `newsletter.campaign_invalid_transition`.", body = Problem),
        (status = 412, description = "The campaign changed since you read it; code `precondition_failed`.", body = Problem),
    )
)]
pub(crate) async fn cancel_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
) -> Result<Tagged<CampaignResponse>, Error> {
    let expected = if_match.expected()?;
    let campaign = service.change_state(&principal, id, Action::Cancel, expected).await?;
    Ok(Tagged { version: campaign.row.updated_at, data: campaign.into() })
}

/// Send a test mail of a campaign.
///
/// Requires `newsletter.send`. Sends the campaign, as subscribers would get it, to one
/// address. Nothing is recorded; the unsubscribe link of a test mail does nothing.
#[utoipa::path(
    post,
    path = "/campaigns/{id}/test",
    extensions(("x-permissions" = json!(["newsletter.send"]))),
    operation_id = "test_newsletter_campaign",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the campaign.")),
    request_body = TestRequest,
    responses(
        (status = 202, description = "The test mail is queued."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `newsletter.send`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No campaign (or no list) has this id; code `newsletter.campaign_not_found` or `newsletter.list_not_found`.", body = Problem),
        (status = 422, description = "`email` is not an email address; see `errors`.", body = Problem),
        (status = 503, description = "A setting needed to send is missing; codes `public_url_missing`, `newsletter.preferences_url_missing`, `newsletter.postal_address_missing`.", body = Problem),
    )
)]
pub(crate) async fn test_campaign(
    State(service): State<Arc<NewsletterService>>,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<TestRequest>,
) -> Result<StatusCode, Error> {
    service.send_test(&principal, id, &input.email).await?;
    Ok(StatusCode::ACCEPTED)
}
