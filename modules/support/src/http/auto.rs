//! Trả lời tự động (D-230): duyệt câu trả lời mẫu, xem thử, nhật ký, phản hồi và "gặp nhân viên".

use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::extras::MacroResponse;
use super::views::TicketResponse;
use crate::api::{AutoSource, ReplyDecision};
use crate::autoreply_admin::AutoReply;
use crate::permissions::AUTO_REPLY_MANAGE;
use crate::service::{SupportService, Viewer};

/// Approval of a saved reply for automatic answers.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ApproveSupportMacroRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ApproveRequest {
    /// `true`: you approve the exact text for automatic answers; `false`: stop using it.
    enabled: bool,
    /// Ways customers ask the question, per language (1 to 50 in all).
    #[serde(default)]
    #[schema(value_type = Object, example = json!({"vi": ["Mấy giờ mở cửa?"], "en": ["When do you open?"]}))]
    questions: BTreeMap<String, Vec<String>>,
    /// Automatic use stops after this day unless approved again (today to one year ahead).
    #[schema(value_type = Option<String>, example = "2027-05-01")]
    review_due_on: Option<NaiveDate>,
}

/// Approve a saved reply for automatic answers, or stop using it.
///
/// Requires `support.auto_reply.manage` at the saved reply's unit. Enabling it means you approve
/// the exact text: it is sent word for word (variables filled in) when the classifier is confident
/// and no rule blocks it. Changing the text later withdraws the approval.
#[utoipa::path(
    put,
    path = "/macros/{id}/auto-reply",
    extensions(("x-permissions" = json!(["support.auto_reply.manage"]))),
    operation_id = "approve_support_macro",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the saved reply.")),
    request_body = ApproveRequest,
    responses(
        (status = 200, description = "The saved reply.", body = Data<MacroResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.auto_reply.manage` (`permission_denied`), or it does not cover the unit (`org_out_of_scope`, `org_frozen`).", body = Problem),
        (status = 404, description = "No saved reply with this id; code `support.saved_reply_not_found`.", body = Problem),
        (status = 422, description = "No example questions (see `errors`), or no valid review day (`support.macro_not_approved`).", body = Problem),
    )
)]
pub(crate) async fn approve_macro(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ApproveRequest>,
) -> Result<Data<MacroResponse>, Error> {
    principal.require(&AUTO_REPLY_MANAGE)?;
    let row = service
        .approve_reply(&tenant, &principal, id, input.enabled, input.questions, input.review_due_on)
        .await?;
    Ok(Data { data: row.into() })
}

/// A question to try.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = PreviewSupportAutoReplyRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewRequest {
    org_id: Uuid,
    text: String,
    /// Default `vi`.
    locale: Option<String>,
}

/// What an automatic answer would do.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportAutoReplyDecision)]
pub(crate) struct DecisionResponse {
    /// `send` (answer automatically), `draft` (staff answer, a draft is prepared) or `handoff`
    /// (staff answer).
    outcome: String,
    /// The exact text that would be sent.
    text: Option<String>,
    /// `macro` or `fact`.
    source: Option<String>,
    macro_id: Option<Uuid>,
    intent: Option<String>,
}

impl From<ReplyDecision> for DecisionResponse {
    fn from(decision: ReplyDecision) -> Self {
        match decision {
            ReplyDecision::Send { text, source } => {
                let (kind, macro_id, intent) = match source {
                    AutoSource::Macro(id) => ("macro", Some(id), None),
                    AutoSource::Fact { intent } => ("fact", None, Some(intent)),
                };
                DecisionResponse {
                    outcome: "send".to_owned(),
                    text: Some(text),
                    source: Some(kind.to_owned()),
                    macro_id,
                    intent,
                }
            }
            ReplyDecision::Draft { .. } => DecisionResponse {
                outcome: "draft".to_owned(),
                text: None,
                source: None,
                macro_id: None,
                intent: None,
            },
            ReplyDecision::Handoff => DecisionResponse {
                outcome: "handoff".to_owned(),
                text: None,
                source: None,
                macro_id: None,
                intent: None,
            },
        }
    }
}

/// Try the automatic answer without sending anything.
///
/// Requires `support.auto_reply.manage` at the unit. Runs the classifier and every rule on `text`
/// as if a customer asked it at the unit, and tells what would happen.
#[utoipa::path(
    post,
    path = "/auto-reply/preview",
    extensions(("x-permissions" = json!(["support.auto_reply.manage"]))),
    operation_id = "preview_support_auto_reply",
    security(("bearer" = [])),
    request_body = PreviewRequest,
    responses(
        (status = 200, description = "The decision.", body = Data<DecisionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.auto_reply.manage` (`permission_denied`), or it does not cover the unit (`org_out_of_scope`).", body = Problem),
        (status = 503, description = "AI is not set up; code `support.auto_reply_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn preview(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<PreviewRequest>,
) -> Result<Data<DecisionResponse>, Error> {
    principal.require(&AUTO_REPLY_MANAGE)?;
    let locale = input.locale.unwrap_or_else(|| "vi".to_owned());
    let decision = service.preview(&tenant, &principal, input.org_id, &input.text, &locale).await?;
    Ok(Data { data: decision.into() })
}

/// One automatic answer.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportAutoReply)]
pub(crate) struct AutoReplyResponse {
    id: Uuid,
    org_id: Uuid,
    /// `support.ticket:<id>` or `conversations.thread:<id>`.
    thread: String,
    /// `macro` or `fact`.
    source: String,
    macro_id: Option<Uuid>,
    intent: Option<String>,
    /// The message sent.
    reply_message_id: Option<Uuid>,
    /// `helpful`, `wrong`, or empty.
    feedback: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<AutoReply> for AutoReplyResponse {
    fn from(row: AutoReply) -> Self {
        AutoReplyResponse {
            id: row.id,
            org_id: row.org_id,
            thread: row.thread,
            source: row.source_kind,
            macro_id: row.macro_id,
            intent: row.intent,
            reply_message_id: row.reply_message_id,
            feedback: row.feedback,
            created_at: row.created_at,
        }
    }
}

/// List automatic answers in your scope, with the customers' feedback.
///
/// Requires `support.auto_reply.manage`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/auto-replies",
    extensions(("x-permissions" = json!(["support.auto_reply.manage"]))),
    operation_id = "list_support_auto_replies",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of automatic answers.", body = Page<AutoReplyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.auto_reply.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_auto_replies(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<AutoReplyResponse>, Error> {
    principal.require(&AUTO_REPLY_MANAGE)?;
    let page = service.auto_replies(&tenant, &principal, &PageRequest::try_from(page)?).await?;
    Ok(page.map(AutoReplyResponse::from))
}

/// Feedback on an automatic answer.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SupportAutoReplyFeedbackRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct FeedbackRequest {
    /// Guests: the ticket token.
    token: Option<String>,
    /// `true`: it helped; `false`: it was wrong.
    helpful: bool,
}

/// Tell whether an automatic answer on your ticket helped.
///
/// Any signed-in account, for its own ticket. A saved reply reported wrong too often stops being
/// used automatically.
#[utoipa::path(
    post,
    path = "/me/tickets/{id}/auto-replies/{reply_id}/feedback",
    extensions(("x-permissions" = json!([]))),
    operation_id = "feedback_my_support_auto_reply",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket."), ("reply_id" = Uuid, Path, description = "Id of the automatic answer.")),
    request_body = FeedbackRequest,
    responses(
        (status = 200, description = "Recorded.", body = Data<AutoReplyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such ticket of yours (`support.ticket_not_found`) or automatic answer on it (`support.auto_reply_not_found`).", body = Problem),
    )
)]
pub(crate) async fn feedback_mine(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, reply)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<FeedbackRequest>,
) -> Result<Data<AutoReplyResponse>, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let row = service.feedback(&tenant, viewer, id, reply, input.helpful).await?;
    Ok(Data { data: row.into() })
}

/// Tell whether an automatic answer on a guest ticket helped.
///
/// Public, with the ticket token.
#[utoipa::path(
    post,
    path = "/public/tickets/{id}/auto-replies/{reply_id}/feedback",
    extensions(("x-public" = json!(true))),
    operation_id = "feedback_public_support_auto_reply",
    security(()),
    params(("id" = Uuid, Path, description = "Id of the ticket."), ("reply_id" = Uuid, Path, description = "Id of the automatic answer.")),
    request_body = FeedbackRequest,
    responses(
        (status = 200, description = "Recorded.", body = Data<AutoReplyResponse>),
        (status = 404, description = "No ticket with this id and token (`support.ticket_not_found`) or automatic answer on it (`support.auto_reply_not_found`).", body = Problem),
    )
)]
pub(crate) async fn feedback_public(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    Path((id, reply)): Path<(Uuid, Uuid)>,
    ValidJson(input): ValidJson<FeedbackRequest>,
) -> Result<Data<AutoReplyResponse>, Error> {
    service.by_token(&tenant, id, input.token.as_deref().unwrap_or_default()).await?;
    let row = service.feedback(&tenant, Viewer::Guest, id, reply, input.helpful).await?;
    Ok(Data { data: row.into() })
}

/// The ticket token of a guest (empty for signed-in customers).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SupportHandoffRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandoffRequest {
    token: Option<String>,
}

/// Ask for a person on your ticket.
///
/// Any signed-in account, for its own ticket. Automatic answers stop on this ticket and it waits
/// for staff.
#[utoipa::path(
    post,
    path = "/me/tickets/{id}/handoff",
    extensions(("x-permissions" = json!([]))),
    operation_id = "handoff_my_support_ticket",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = HandoffRequest,
    responses(
        (status = 200, description = "The ticket.", body = Data<TicketResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no ticket with this id; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is closed; code `support.ticket_closed`.", body = Problem),
    )
)]
pub(crate) async fn handoff_mine(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(_input): ValidJson<HandoffRequest>,
) -> Result<Data<TicketResponse>, Error> {
    let ticket = service.handoff(&tenant, Viewer::Customer(principal.user_id()), id).await?;
    Ok(Data { data: TicketResponse::new(ticket, false) })
}

/// Ask for a person on a guest ticket.
///
/// Public, with the ticket token. Automatic answers stop on this ticket and it waits for staff.
#[utoipa::path(
    post,
    path = "/public/tickets/{id}/handoff",
    extensions(("x-public" = json!(true))),
    operation_id = "handoff_public_support_ticket",
    security(()),
    params(("id" = Uuid, Path, description = "Id of the ticket.")),
    request_body = HandoffRequest,
    responses(
        (status = 200, description = "The ticket.", body = Data<TicketResponse>),
        (status = 404, description = "No ticket with this id and token; code `support.ticket_not_found`.", body = Problem),
        (status = 409, description = "The ticket is closed; code `support.ticket_closed`.", body = Problem),
    )
)]
pub(crate) async fn handoff_public(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<HandoffRequest>,
) -> Result<Data<TicketResponse>, Error> {
    service.by_token(&tenant, id, input.token.as_deref().unwrap_or_default()).await?;
    let ticket = service.handoff(&tenant, Viewer::Guest, id).await?;
    Ok(Data { data: TicketResponse::new(ticket, false) })
}
