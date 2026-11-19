//! Endpoint của người mua: cuộc trò chuyện của chính mình. Tài khoản người mua (`customer`) không có
//! permission (bước 34): chỉ cần đăng nhập, và chỉ chạm cuộc có `customer_user_id` là mình.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::views::{
    MessageResponse, ReadResponse, ReportResponse, StartedResponse, ThreadDetail, ThreadResponse,
};
use super::{MessageRequest, ReadRequest, ReportRequest, sent};
use crate::messages::NewMessage;
use crate::service::{ConversationsService, Viewer};
use crate::threads::NewThread;

/// The first message of a conversation.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = StartConversationRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartRequest {
    /// The unit (shop, marketplace store) to write to. Omitted: the shop itself (root unit).
    org_id: Option<Uuid>,
    /// The record you ask about, `<kind>:<id>` (e.g. `catalog.product:<id>`, `orders.order:<id>`).
    #[schema(example = "catalog.product:0190f5a4-8a5e-7c3e-9b1a-2f7d4c1e8a10")]
    subject: Option<String>,
    /// Plain text; may be empty when you send images.
    #[serde(default)]
    body: String,
    /// An id your app makes up for this message (1 to 100 characters): sending again with the same
    /// id does not add a second message.
    client_id: String,
    /// Images you uploaded yourself (at most 5).
    #[serde(default)]
    media_ids: Vec<Uuid>,
    /// Your language (`vi`, `en`) for answers and notices.
    locale: Option<String>,
}

/// Start a conversation with a shop.
///
/// Any signed-in account. When you already have an open conversation with the same unit about the
/// same record, the message goes there (200) instead of starting another. The shop may ask for a
/// verified account first (`conversations.verification_required`, Decree 147/2024) or may have
/// blocked you (`conversations.blocked`).
#[utoipa::path(
    post,
    path = "/me/threads",
    extensions(("x-permissions" = json!([]))),
    operation_id = "start_my_conversation",
    security(("bearer" = [])),
    request_body = StartRequest,
    responses(
        (status = 201, description = "Started.", body = Data<StartedResponse>),
        (status = 200, description = "Written into your open conversation with this unit about this record.", body = Data<StartedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The shop blocked you (`conversations.blocked`) or asks for a verified account (`conversations.verification_required`).", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or an image was not uploaded by you (`conversations.attachment_not_owned`).", body = Problem),
        (status = 429, description = "You send messages too fast; code `conversations.too_many_messages`.", body = Problem),
    )
)]
pub(crate) async fn start(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<StartRequest>,
) -> Result<Response, Error> {
    let new = NewThread {
        org_id: input.org_id,
        subject: input.subject,
        message: NewMessage {
            body: input.body,
            client_id: input.client_id,
            media_ids: input.media_ids,
        },
        locale: input.locale,
    };
    let started = service.start(&tenant, principal.user_id(), new).await?;
    let id = started.thread.id;
    let data = StartedResponse {
        thread: ThreadResponse::new(started.thread, false),
        message: started.message.into(),
    };
    Ok(if started.created {
        Created::new(format!("/conversations/me/threads/{id}"), data).into_response()
    } else {
        Data { data }.into_response()
    })
}

/// List your conversations.
///
/// Any signed-in account. Latest activity first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/me/threads",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_conversations",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of your conversations.", body = Page<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_mine(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<ThreadResponse>, Error> {
    let request = PageRequest::try_from(page)?;
    let page = service.list_mine(&tenant, principal.user_id(), &request).await?;
    Ok(page.map(|thread| ThreadResponse::new(thread, false)))
}

/// Get one of your conversations.
///
/// Any signed-in account; a conversation of someone else answers 404. Includes how far each person
/// has read.
#[utoipa::path(
    get,
    path = "/me/threads/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "The conversation.", body = Data<ThreadDetail>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_mine(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ThreadDetail>, Error> {
    let thread = service.get(&tenant, Viewer::Customer(principal.user_id()), id).await?;
    let reads = service.reads(&tenant, &thread).await?;
    Ok(Data {
        data: ThreadDetail {
            thread: ThreadResponse::new(thread, false),
            reads: reads.into_iter().map(ReadResponse::from).collect(),
        },
    })
}

/// List the messages of one of your conversations.
///
/// Any signed-in account. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/me/threads/{id}/messages",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_conversation_messages",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation."), PageQuery),
    responses(
        (status = 200, description = "One page of messages.", body = Page<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_my_messages(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Page<MessageResponse>, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let (_, page) = service.messages(&tenant, viewer, id, &PageRequest::try_from(page)?).await?;
    Ok(page.map(MessageResponse::from))
}

/// Write in one of your conversations.
///
/// Any signed-in account. Sending again with the same `client_id` returns the first message (200)
/// instead of adding another. Simple questions may get an automatic answer (marked `auto_reply`).
#[utoipa::path(
    post,
    path = "/me/threads/{id}/messages",
    extensions(("x-permissions" = json!([]))),
    operation_id = "add_my_conversation_message",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = MessageRequest,
    responses(
        (status = 201, description = "Sent.", body = Data<MessageResponse>),
        (status = 200, description = "A message with this `client_id` was already sent; this is it.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The shop blocked you (`conversations.blocked`) or asks for a verified account (`conversations.verification_required`).", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
        (status = 409, description = "The conversation is closed; start a new one. Code `conversations.thread_closed`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or an image was not uploaded by you (`conversations.attachment_not_owned`).", body = Problem),
        (status = 429, description = "You send messages too fast; code `conversations.too_many_messages`.", body = Problem),
    )
)]
pub(crate) async fn add_my_message(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<MessageRequest>,
) -> Result<Response, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let (message, created) = service.send(&tenant, viewer, id, NewMessage::from(input)).await?;
    Ok(sent(format!("/conversations/me/threads/{id}"), message, created))
}

/// Mark one of your conversations as read.
///
/// Any signed-in account. Moves your read cursor up to `message_id` (default: the newest message);
/// the shop sees how far you have read.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/read",
    extensions(("x-permissions" = json!([]))),
    operation_id = "read_my_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = ReadRequest,
    responses(
        (status = 200, description = "The conversation after reading.", body = Data<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "`message_id` is not a message of this conversation; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn read_mine(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReadRequest>,
) -> Result<Data<ThreadResponse>, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let thread = service.mark_read(&tenant, viewer, id, input.message_id).await?;
    Ok(Data { data: ThreadResponse::new(thread, false) })
}

/// Tell the shop you are typing.
///
/// Any signed-in account. Sends the event `conversations.typing`; more than one call per 3 seconds
/// is ignored. Nothing is stored.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/typing",
    extensions(("x-permissions" = json!([]))),
    operation_id = "type_in_my_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 204, description = "Sent (or ignored, when called again within 3 seconds)."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
        (status = 409, description = "The conversation is closed; code `conversations.thread_closed`.", body = Problem),
    )
)]
pub(crate) async fn typing_mine(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.typing(&tenant, Viewer::Customer(principal.user_id()), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Recall a message you sent.
///
/// Any signed-in account; only your own message, within 2 minutes of sending it. Its text and images
/// are removed, its place stays marked as recalled.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/messages/{message_id}/recall",
    extensions(("x-permissions" = json!([]))),
    operation_id = "recall_my_conversation_message",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the conversation."),
        ("message_id" = Uuid, Path, description = "Id of your message."),
    ),
    responses(
        (status = 200, description = "Recalled.", body = Data<MessageResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such conversation of yours (`conversations.thread_not_found`) or no such message of yours (`conversations.message_not_found`).", body = Problem),
        (status = 409, description = "More than 2 minutes have passed; code `conversations.recall_expired`.", body = Problem),
    )
)]
pub(crate) async fn recall_mine(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, message_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<MessageResponse>, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let message = service.recall(&tenant, viewer, id, message_id).await?;
    Ok(Data { data: message.into() })
}

/// Close one of your conversations.
///
/// Any signed-in account. Writing again later starts a new conversation.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/close",
    extensions(("x-permissions" = json!([]))),
    operation_id = "close_my_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "Closed.", body = Data<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
    )
)]
pub(crate) async fn close_mine(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ThreadResponse>, Error> {
    let thread = service.close(&tenant, Viewer::Customer(principal.user_id()), id).await?;
    Ok(Data { data: ThreadResponse::new(thread, false) })
}

/// Talk to a person.
///
/// Any signed-in account. Stops automatic answers in this conversation and marks it as waiting for
/// staff (`needs_agent`). Calling again changes nothing.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/handoff",
    extensions(("x-permissions" = json!([]))),
    operation_id = "hand_off_my_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "Waiting for a person.", body = Data<ThreadResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
        (status = 409, description = "The conversation is closed; code `conversations.thread_closed`.", body = Problem),
    )
)]
pub(crate) async fn handoff(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<ThreadResponse>, Error> {
    let thread = service.handoff(&tenant, principal.user_id(), id).await?;
    Ok(Data { data: ThreadResponse::new(thread, false) })
}

/// Report one of your conversations, or a message in it, as abusive.
///
/// Any signed-in account. The shop's managers review it.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/reports",
    extensions(("x-permissions" = json!([]))),
    operation_id = "report_my_conversation",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = ReportRequest,
    responses(
        (status = 201, description = "Reported.", body = Data<ReportResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn report_mine(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<ReportRequest>,
) -> Result<Created<ReportResponse>, Error> {
    let viewer = Viewer::Customer(principal.user_id());
    let row = service.report(&tenant, viewer, id, input.message_id, &input.reason).await?;
    Ok(Created::new(format!("/conversations/me/threads/{id}"), row.into()))
}
