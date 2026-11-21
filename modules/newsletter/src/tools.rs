//! Tool MCP của module `newsletter`. Mỗi tool: một kiểu đầu vào, permission như endpoint REST
//! tương ứng (service tự kiểm), thân là lời gọi service như REST (luật kiến trúc 8). Agent có
//! `newsletter.manage` soạn được chiến dịch nháp; gửi và hẹn giờ cần thêm `newsletter.send`.
//!
//! Kiểu đầu vào riêng cho MCP, chỉ trường phẳng (JSON Schema của tool không `$ref`). Tool gửi /
//! hẹn giờ nhận `version` — giá trị `ETag` tool đọc trả về — y như `If-Match` của REST.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, etag};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::audience::subscribers::SubscriberFilter;
use crate::campaigns::{Action, Campaign, CampaignFormat, CampaignInput};
use crate::http::{CampaignResponse, ListResponse, SubscriberResponse, choice, page_request};
use crate::permissions::{NEWSLETTER_MANAGE, NEWSLETTER_READ, NEWSLETTER_SEND};
use crate::service::NewsletterService;

/// Chiến dịch kèm `version` (giá trị `ETag`) để gửi lại khi gửi / hẹn giờ.
#[derive(Debug, Serialize)]
struct Versioned {
    /// Gửi lại trong `version` của tool gửi / hẹn giờ.
    version: String,
    #[serde(flatten)]
    campaign: CampaignResponse,
}

fn versioned(found: Campaign) -> Versioned {
    Versioned { version: etag(found.row.updated_at), campaign: found.into() }
}

/// Paging of the list of lists.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListListsInput {
    /// Maximum number of lists to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// Filters of a subscriber search.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindSubscribersInput {
    /// Part of the email address, any case.
    email: Option<String>,
    /// Only subscribers of this list.
    list_id: Option<Uuid>,
    /// `unconfirmed`, `confirmed` or `unsubscribed` (in `list_id`, or in any list).
    status: Option<String>,
    /// `true`: only blocked subscribers; `false`: only the others.
    blocked: Option<bool>,
    /// Maximum number of subscribers to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// A new draft campaign.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateCampaignInput {
    /// The list to send to.
    list_id: Uuid,
    /// 1 to 200 characters, one line.
    subject: String,
    /// Short text shown after the subject in some mail apps (HTML mails only).
    preheader: Option<String>,
    /// Plain-text body. A footer with the unsubscribe link is added to every mail.
    body_text: String,
    /// HTML body, sent as is (not sanitised: admin interfaces preview it in a sandboxed iframe).
    body_html: Option<String>,
    /// Language of the footer: `en` (default) or `vi`.
    locale: Option<String>,
    /// An advertising mail (labelled, limited per day per address).
    advertising: Option<bool>,
    /// `static` (default) or `template` (`{{ subscriber.name }}`, `{{ subscriber.email }}`,
    /// `{{ list.name }}`, `{{ unsubscribe_url }}`).
    format: Option<String>,
}

/// A campaign to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CampaignId {
    /// Id of the campaign.
    id: Uuid,
}

/// A campaign to send now.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SendInput {
    /// Id of the campaign.
    id: Uuid,
    /// The `version` you read (or `*`). Refused with `precondition_failed` if the campaign
    /// changed since: read it again, check it, and retry.
    version: String,
}

/// A campaign to schedule.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ScheduleInput {
    /// Id of the campaign.
    id: Uuid,
    /// When to start sending: a future time with a time zone, e.g. `2026-12-01T08:00:00Z`.
    at: Timestamp,
    /// The `version` you read (or `*`).
    version: String,
}

pub(crate) fn all(service: &Arc<NewsletterService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (lists, find, create, get, send, schedule) = (
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
        service.clone(),
    );
    vec![
        Tool::new(
            "list_lists",
            "List the newsletter lists, newest first, with how many people are unconfirmed, \
             confirmed and unsubscribed. Follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: ListListsInput| {
                let service = lists.clone();
                async move {
                    let paging = PageQuery::new(
                        input.limit.map(|limit| limit.to_string()),
                        input.cursor,
                        None,
                    );
                    let page = page_request(paging, Vec::new())?;
                    let found = service.list_lists(&tenant, &principal, &page).await?;
                    let ids: Vec<Uuid> = found.data.iter().map(|list| list.row.id).collect();
                    let mut counts = service.subscription_counts(&tenant, &ids).await?;
                    Ok::<Page<ListResponse>, Error>(found.map(|list| {
                        let count = counts.remove(&list.row.id).unwrap_or_default();
                        ListResponse::new(list, count)
                    }))
                }
            },
        )
        .requires(NEWSLETTER_READ)
        .read_only(),
        Tool::new(
            "find_subscribers",
            "Find subscribers by part of their email address, list or subscription status, \
             newest first, with their subscriptions. Follow `pagination.next_cursor` with the \
             same filters for more.",
            move |tenant: TenantContext, principal: Principal, input: FindSubscribersInput| {
                let service = find.clone();
                async move {
                    let mut problems = Vec::new();
                    let filter = SubscriberFilter {
                        email: input.email,
                        list_id: input.list_id,
                        status: choice(
                            input.status.as_deref(),
                            "status",
                            "`unconfirmed`, `confirmed`, `unsubscribed`",
                            &mut problems,
                        ),
                        blocked: input.blocked,
                        sort: Default::default(),
                    };
                    let paging = PageQuery::new(
                        input.limit.map(|limit| limit.to_string()),
                        input.cursor,
                        None,
                    );
                    let page = page_request(paging, problems)?;
                    let found =
                        service.list_subscribers(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<SubscriberResponse>, Error>(found.map(Into::into))
                }
            },
        )
        .requires(NEWSLETTER_READ)
        .read_only(),
        Tool::new(
            "create_campaign",
            "Write a draft campaign for a list. Nothing is sent: a person (or an agent with \
             `newsletter.send`) sends or schedules it after review. Returns the campaign with \
             its `version`.",
            move |tenant: TenantContext, principal: Principal, input: CreateCampaignInput| {
                let service = create.clone();
                async move {
                    let mut problems = Vec::new();
                    let locale =
                        crate::text::locale_param(input.locale.as_deref(), "locale", &mut problems);
                    let format = match input.format.as_deref() {
                        None | Some("static") => Some(CampaignFormat::Static),
                        Some("template") => Some(CampaignFormat::Template),
                        Some(_) => {
                            problems.push(autoxide_kernel::error::FieldError::new(
                                "format",
                                "invalid",
                                "must be `static` or `template`",
                            ));
                            None
                        }
                    };
                    if !problems.is_empty() {
                        return Err(Error::validation(problems));
                    }
                    let input = CampaignInput {
                        list_id: Some(input.list_id),
                        subject: Some(input.subject),
                        preheader: input.preheader.map(Some),
                        body_text: Some(input.body_text),
                        body_html: input.body_html.map(Some),
                        locale,
                        advertising: input.advertising,
                        format,
                    };
                    Ok::<Versioned, Error>(versioned(
                        service.create_campaign(&tenant, &principal, input).await?,
                    ))
                }
            },
        )
        .requires(NEWSLETTER_MANAGE),
        Tool::new(
            "get_campaign",
            "Get a campaign: content, status and progress (`progress.total`, `pending`, \
             `sent`, `failed`, `skipped`), and the `version` needed to send or schedule it.",
            move |tenant: TenantContext, principal: Principal, input: CampaignId| {
                let service = get.clone();
                async move {
                    Ok::<Versioned, Error>(versioned(
                        service.get_campaign(&tenant, &principal, input.id).await?,
                    ))
                }
            },
        )
        .requires(NEWSLETTER_READ)
        .read_only(),
        Tool::new(
            "send_campaign",
            "Send a draft or scheduled campaign now to every confirmed subscriber of its list. \
             Mails go out in batches; follow the progress with `get_campaign`. Pass the \
             `version` you read.",
            move |tenant: TenantContext, principal: Principal, input: SendInput| {
                let service = send.clone();
                async move {
                    let expected = IfMatch::from_value(&input.version).expected()?;
                    let found = service
                        .change_state(&tenant, &principal, input.id, Action::Send, expected)
                        .await?;
                    Ok::<Versioned, Error>(versioned(found))
                }
            },
        )
        .requires(NEWSLETTER_SEND),
        Tool::new(
            "schedule_campaign",
            "Schedule a draft or scheduled campaign to start sending at `at`. Pass the \
             `version` you read.",
            move |tenant: TenantContext, principal: Principal, input: ScheduleInput| {
                let service = schedule.clone();
                async move {
                    if input.version.trim().is_empty() {
                        return Err(Error::validation(vec![FieldError::new(
                            "version",
                            "required",
                            "is required",
                        )]));
                    }
                    let expected = IfMatch::from_value(&input.version).expected()?;
                    let found = service
                        .change_state(
                            &tenant,
                            &principal,
                            input.id,
                            Action::Schedule(input.at),
                            expected,
                        )
                        .await?;
                    Ok(versioned(found))
                }
            },
        )
        .requires(NEWSLETTER_SEND),
    ]
}
