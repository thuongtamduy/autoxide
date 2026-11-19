//! Endpoint của nhân viên: hộp thư chung, một cuộc, tin, trả lời, đã đọc, đang gõ, thu hồi, phân
//! công, đóng, chuyển phiếu, bản nháp AI, chặn, báo vi phạm.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use super::views::{
    BlockResponse, EscalatedResponse, MessageResponse, ReadResponse, ReportResponse, ThreadDetail,
    ThreadResponse,
};
use super::{MessageRequest, ReadRequest, ReportRequest, sent};
use crate::messages::NewMessage;
use crate::permissions::{MANAGE, READ, REPLY};
use crate::service::{ConversationsService, Viewer};
use crate::threads::ThreadFilter;

/// Filters of the conversation list; all optional and combined with AND.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ThreadQuery {
    /// `open` or `closed`.
    status: Option<String>,
    /// `me`, `none` or a user id.
    assignee: Option<String>,
    /// `true`: only conversations where the customer wrote and no agent has read it yet.
    unread: Option<bool>,
    /// `true`: only conversations where the customer asked for a person.
    needs_agent: Option<bool>,
    /// Only conversations of this unit (not the units below it).
    org_id: Option<Uuid>,
    /// Only conversations about this record, `<kind>:<id>`.
    subject: Option<String>,
}

impl ThreadQuery {
    fn filter(self, principal: &Principal) -> Result<ThreadFilter, Error> {
        let assignee = match self.assignee.as_deref() {
            None => None,
            Some("me") => Some(Some(principal.user_id())),
            Some("none") => Some(None),
            Some(raw) => Some(Some(raw.parse::<Uuid>().map_err(|_| {
                Error::validation(vec![FieldError::new(
                    "assignee",
                    "invalid",
                    "must be `me`, `none` or a user id",
                )])
            })?)),
        };
        if let Some(status) = &self.status
            && !matches!(status.as_str(), "open" | "closed")
        {
            return Err(Error::validation(vec![FieldError::new(
                "status",
                "invalid",
                "must be `open` or `closed`",
            )]));
        }
        Ok(ThreadFilter {
            status: self.status,
            assignee,
            unread: self.unread,
            needs_agent: self.needs_agent,
            org_id: self.org_id,
            subject: self.subject,
        })
    }
}

/// List the customer conversations in your scope (shared inbox).
///
/// Requires `conversations.read`; its scope decides which units' conversations you see (`unit`,
/// `subtree`, `tenant`; `own`: conversations assigned to you). Latest activity first unless
/// `order=asc`.
#[utoipa::path(
    get,
    path = "/threads",
    extensions(("x-permissions" = json!(["conversations.read"]))),
    operation_id = "list_conversations",
    security(("bearer" = [])),
    params(ThreadQuery, PageQuery),
    responses(
        (status = 200, description = "One page of conversations.", body = Page<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_threads(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ThreadQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ThreadResponse>, Error> {
    principal.require(&READ)?;
    let filter = query.filter(&principal)?;
    let page = service.list(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(|thread| ThreadResponse::new(thread, true)))
}

/// Get one customer conversation in your scope.
///
/// Requires `conversations.read` at the unit of the conversation. Includes how far each person has
/// read; messages come from `GET /threads/{id}/messages`.
#[utoipa::path(
    get,
    path = "/threads/{id}",
    extensions(("x-permissions" = json!(["conversations.read"]))),
    operation_id = "get_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "The conversation.", body = Data<ThreadDetail>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_thread(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ThreadDetail>, Error> {
    principal.require(&READ)?;
    let thread = service.get(&tenant, Viewer::Staff(&principal), id).await?;
    let reads = service.reads(&tenant, &thread).await?;
    Ok(Data {
        data: ThreadDetail {
            thread: ThreadResponse::new(thread, true),
            reads: reads.into_iter().map(ReadResponse::from).collect(),
        },
    })
}

/// List the messages of a customer conversation in your scope.
///
/// Requires `conversations.read`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/threads/{id}/messages",
    extensions(("x-permissions" = json!(["conversations.read"]))),
    operation_id = "list_conversation_messages",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation."), PageQuery),
    responses(
        (status = 200, description = "One page of messages.", body = Page<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_messages(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<MessageResponse>, Error> {
    principal.require(&READ)?;
    let viewer = Viewer::Staff(&principal);
    let (_, page) = service.messages(&tenant, viewer, id, &PageRequest::try_from(page)?).await?;
    Ok(page.map(MessageResponse::from))
}

/// Answer the customer of a conversation.
///
/// Requires `conversations.reply` at the unit of the conversation. Sending again with the same
/// `client_id` returns the first message (200) instead of adding another. The customer gets the
/// message in real time, or a notice when they do not read it within a few minutes.
#[utoipa::path(
    post,
    path = "/threads/{id}/messages",
    extensions(("x-permissions" = json!(["conversations.reply"]))),
    operation_id = "reply_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = MessageRequest,
    responses(
        (status = 201, description = "Sent.", body = Data<MessageResponse>),
        (status = 200, description = "A message with this `client_id` was already sent; this is it.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.reply`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 409, description = "The conversation is closed; code `conversations.thread_closed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or an image is not in the media library (`conversations.attachment_not_owned`).", body = Problem),
        (status = 429, description = "You send messages too fast; code `conversations.too_many_messages`.", body = Problem),
    )
)]
pub(crate) async fn reply(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MessageRequest>,
) -> Result<Response, Error> {
    principal.require(&REPLY)?;
    let new = NewMessage::from(input);
    let (message, created) = service.send(&tenant, Viewer::Staff(&principal), id, new).await?;
    Ok(sent(format!("/conversations/threads/{id}"), message, created))
}

/// Mark a conversation as read.
///
/// Requires `conversations.read`. Moves your read cursor up to `message_id` (default: the newest
/// message; it never moves back) and clears the unread count of staff when you reach the newest
/// message. The customer sees how far staff have read.
#[utoipa::path(
    post,
    path = "/threads/{id}/read",
    extensions(("x-permissions" = json!(["conversations.read"]))),
    operation_id = "read_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = ReadRequest,
    responses(
        (status = 200, description = "The conversation after reading.", body = Data<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "`message_id` is not a message of this conversation; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn read(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReadRequest>,
) -> Result<Data<ThreadResponse>, Error> {
    principal.require(&READ)?;
    let thread =
        service.mark_read(&tenant, Viewer::Staff(&principal), id, input.message_id).await?;
    Ok(Data { data: ThreadResponse::new(thread, true) })
}

/// Tell the customer you are typing.
///
/// Requires `conversations.read`. Sends the event `conversations.typing` on the event stream; more
/// than one call per 3 seconds is ignored. Nothing is stored.
#[utoipa::path(
    post,
    path = "/threads/{id}/typing",
    extensions(("x-permissions" = json!(["conversations.read"]))),
    operation_id = "type_in_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 204, description = "Sent (or ignored, when called again within 3 seconds)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 409, description = "The conversation is closed; code `conversations.thread_closed`.", body = Problem),
    )
)]
pub(crate) async fn typing(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    principal.require(&READ)?;
    service.typing(&tenant, Viewer::Staff(&principal), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Recall a message you sent.
///
/// Requires `conversations.reply`. Only your own message, within 2 minutes of sending it: its text
/// and images are removed, its place stays marked as recalled. Recalling again changes nothing.
#[utoipa::path(
    post,
    path = "/threads/{id}/messages/{message_id}/recall",
    extensions(("x-permissions" = json!(["conversations.reply"]))),
    operation_id = "recall_conversation_message",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the conversation."),
        ("message_id" = Uuid, Path, description = "Id of your message."),
    ),
    responses(
        (status = 200, description = "Recalled.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.reply`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such conversation in your scope (`conversations.thread_not_found`) or no such message of yours (`conversations.message_not_found`).", body = Problem),
        (status = 409, description = "More than 2 minutes have passed; code `conversations.recall_expired`.", body = Problem),
    )
)]
pub(crate) async fn recall(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, message_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<MessageResponse>, Error> {
    principal.require(&REPLY)?;
    let message = service.recall(&tenant, Viewer::Staff(&principal), id, message_id).await?;
    Ok(Data { data: message.into() })
}

/// Who answers a conversation.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = AssignConversationRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct AssignRequest {
    /// The colleague to assign; `null` to leave it to the whole team.
    assignee_user_id: Option<Uuid>,
}

/// Assign a conversation.
///
/// Taking it yourself (or letting go of your own) requires `conversations.reply`; assigning a
/// colleague requires `conversations.manage` — the colleague needs `conversations.reply` at the
/// unit of the conversation.
#[utoipa::path(
    post,
    path = "/threads/{id}/assign",
    extensions(("x-permissions" = json!(["conversations.reply", "conversations.manage"]))),
    operation_id = "assign_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = AssignRequest,
    responses(
        (status = 200, description = "Assigned.", body = Data<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.reply` or `conversations.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "The colleague cannot answer conversations of this unit; code `conversations.assignee_out_of_scope`.", body = Problem),
    )
)]
pub(crate) async fn assign(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AssignRequest>,
) -> Result<Data<ThreadResponse>, Error> {
    principal.require(&REPLY)?;
    let thread = service.assign(&tenant, &principal, id, input.assignee_user_id).await?;
    Ok(Data { data: ThreadResponse::new(thread, true) })
}

/// Close a conversation.
///
/// Requires `conversations.reply`. A closed conversation takes no more messages; when the customer
/// writes again, a new conversation starts. Closing again changes nothing.
#[utoipa::path(
    post,
    path = "/threads/{id}/close",
    extensions(("x-permissions" = json!(["conversations.reply"]))),
    operation_id = "close_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "Closed.", body = Data<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.reply`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
    )
)]
pub(crate) async fn close(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ThreadResponse>, Error> {
    principal.require(&REPLY)?;
    let thread = service.close(&tenant, Viewer::Staff(&principal), id).await?;
    Ok(Data { data: ThreadResponse::new(thread, true) })
}

/// Hand a conversation over as a support ticket.
///
/// Requires `conversations.reply`. Opens a support ticket for the customer with the latest
/// messages and a link back to the conversation (module `support`). Calling again returns the same
/// ticket.
#[utoipa::path(
    post,
    path = "/threads/{id}/escalate",
    extensions(("x-permissions" = json!(["conversations.reply"]))),
    operation_id = "escalate_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "The ticket.", body = Data<EscalatedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.reply`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 409, description = "The customer deleted their account; code `conversations.customer_deleted`.", body = Problem),
        (status = 503, description = "Module `support` is not installed; code `conversations.support_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn escalate(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<EscalatedResponse>, Error> {
    principal.require(&REPLY)?;
    let (thread, ticket) = service.escalate(&tenant, &principal, id).await?;
    Ok(Data {
        data: EscalatedResponse {
            thread: ThreadResponse::new(thread, true),
            ticket_id: ticket.id,
            ticket_number: ticket.number,
        },
    })
}

/// Ask the AI for a draft answer.
///
/// Requires `conversations.reply`. The draft is stored on the conversation (`draft_reply`) for you
/// to edit and send; nothing is sent to the customer.
#[utoipa::path(
    post,
    path = "/threads/{id}/suggest-reply",
    extensions(("x-permissions" = json!(["conversations.reply"]))),
    operation_id = "suggest_conversation_reply",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "The conversation with its new draft.", body = Data<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.reply`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 503, description = "AI is not set up; code `conversations.ai_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn suggest_reply(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ThreadResponse>, Error> {
    principal.require(&REPLY)?;
    let thread = service.suggest_reply(&tenant, &principal, id).await?;
    Ok(Data { data: ThreadResponse::new(thread, true) })
}

/// Why a customer is blocked.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = BlockConversationCustomerRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct BlockRequest {
    /// Up to 500 characters, kept for colleagues (never shown to the customer).
    #[serde(default)]
    reason: String,
}

/// Block the customer of a conversation.
///
/// Requires `conversations.manage`. The customer can no longer write to the unit of this
/// conversation (`conversations.blocked`). Blocking again changes nothing.
#[utoipa::path(
    put,
    path = "/threads/{id}/block",
    extensions(("x-permissions" = json!(["conversations.manage"]))),
    operation_id = "block_conversation_customer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = BlockRequest,
    responses(
        (status = 200, description = "Blocked.", body = Data<BlockResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "The reason is too long; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn block(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<BlockRequest>,
) -> Result<Data<BlockResponse>, Error> {
    principal.require(&MANAGE)?;
    let blocked = service.set_block(&tenant, &principal, id, true, &input.reason).await?;
    Ok(Data { data: BlockResponse { blocked } })
}

/// Lift the block of the customer of a conversation.
///
/// Requires `conversations.manage`. Lifting again changes nothing.
#[utoipa::path(
    delete,
    path = "/threads/{id}/block",
    extensions(("x-permissions" = json!(["conversations.manage"]))),
    operation_id = "unblock_conversation_customer",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "Not blocked any more.", body = Data<BlockResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
    )
)]
pub(crate) async fn unblock(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<BlockResponse>, Error> {
    principal.require(&MANAGE)?;
    let blocked = service.set_block(&tenant, &principal, id, false, "").await?;
    Ok(Data { data: BlockResponse { blocked } })
}

/// Report a conversation or a message as abusive.
///
/// Requires `conversations.read`. Colleagues with `conversations.manage` review reports at
/// `GET /reports`.
#[utoipa::path(
    post,
    path = "/threads/{id}/reports",
    extensions(("x-permissions" = json!(["conversations.read"]))),
    operation_id = "report_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = ReportRequest,
    responses(
        (status = 201, description = "Reported.", body = Data<ReportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn report(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReportRequest>,
) -> Result<autoxide_kernel::response::Created<ReportResponse>, Error> {
    principal.require(&READ)?;
    let viewer = Viewer::Staff(&principal);
    let row = service.report(&tenant, viewer, id, input.message_id, &input.reason).await?;
    Ok(autoxide_kernel::response::Created::new("/conversations/reports", row.into()))
}

/// Filter of the report list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ReportQuery {
    /// `open` or `resolved`.
    status: Option<String>,
}

/// List reports of abuse in your scope.
///
/// Requires `conversations.manage`. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/reports",
    extensions(("x-permissions" = json!(["conversations.manage"]))),
    operation_id = "list_conversation_reports",
    security(("bearer" = [])),
    params(ReportQuery, PageQuery),
    responses(
        (status = 200, description = "One page of reports.", body = Page<ReportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.manage`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_reports(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ReportQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<ReportResponse>, Error> {
    principal.require(&MANAGE)?;
    let request = PageRequest::try_from(page)?;
    let page = service.reports(&tenant, &principal, query.status.as_deref(), &request).await?;
    Ok(page.map(ReportResponse::from))
}

/// Mark a report as reviewed.
///
/// Requires `conversations.manage` at the unit of the report. Calling again changes nothing.
#[utoipa::path(
    post,
    path = "/reports/{id}/resolve",
    extensions(("x-permissions" = json!(["conversations.manage"]))),
    operation_id = "resolve_conversation_report",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the report.")),
    responses(
        (status = 200, description = "Reviewed.", body = Data<ReportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No report with this id in your scope; code `conversations.report_not_found`.", body = Problem),
    )
)]
pub(crate) async fn resolve_report(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ReportResponse>, Error> {
    principal.require(&MANAGE)?;
    let row = service.resolve_report(&tenant, &principal, id).await?;
    Ok(Data { data: row.into() })
}
