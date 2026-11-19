//! Tool MCP của module `conversations`. Mỗi tool: một kiểu đầu vào, permission như endpoint REST
//! tương ứng, thân là lời gọi service — không có logic riêng (luật kiến trúc 8). Không có tool cho
//! người mua.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::views::{MessageResponse, ThreadResponse};
use crate::http::voice::{CallResponse, CallbackResponse};
use crate::messages::NewMessage;
use crate::permissions::{CALL, READ, REPLY};
use crate::service::{ConversationsService, Viewer};
use crate::threads::ThreadFilter;
use crate::voice::callbacks::CallbackFilter;

/// Filters of the conversation list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ListConversationsInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListInput {
    /// `open` or `closed`.
    status: Option<String>,
    /// `true`: only conversations where the customer wrote and no agent has read it yet.
    unread: Option<bool>,
    /// `true`: only conversations where the customer asked for a person.
    needs_agent: Option<bool>,
    /// `true`: only conversations assigned to you.
    #[serde(default)]
    mine: bool,
    /// Maximum number of conversations, 1 to 200 (default 50).
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// A conversation to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ConversationIdInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ThreadId {
    /// Id of the conversation.
    id: Uuid,
}

/// A message to send.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ConversationReplyInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReplyInput {
    /// Id of the conversation.
    id: Uuid,
    /// Plain text.
    body: String,
    /// An id you make up for this message (1 to 100 characters): calling again with the same
    /// `client_id` does not send it twice.
    client_id: String,
}

/// One conversation with its latest messages.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ConversationWithMessages)]
pub(crate) struct ThreadWithMessages {
    thread: ThreadResponse,
    /// The latest 50 messages, oldest first.
    messages: Vec<MessageResponse>,
}

/// Filters of the call-back list.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ListConversationCallbacksInput)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallbacksInput {
    /// `open`, `calling`, `done`, `cancelled` or `expired`.
    status: Option<String>,
    /// Maximum number of requests, 1 to 200 (default 50).
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// The draft for an agent.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ConversationDraft)]
pub(crate) struct DraftResponse {
    /// Text for an agent to edit and send; nothing was sent to the customer.
    draft_reply: Option<String>,
}

pub(crate) fn all(service: &Arc<ConversationsService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (list, get, reply, suggest) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    let (callbacks, calls) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "list_threads",
            "List customer conversations in your scope, latest activity first; follow `pagination.next_cursor` for more.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = list.clone();
                async move {
                    let filter = ThreadFilter {
                        status: input.status,
                        unread: input.unread,
                        needs_agent: input.needs_agent,
                        assignee: input.mine.then_some(Some(principal.user_id())),
                        ..ThreadFilter::default()
                    };
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let page = service.list(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<ThreadResponse>, Error>(
                        page.map(|thread| ThreadResponse::new(thread, true)),
                    )
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "get_thread",
            "Get a customer conversation in your scope with its latest 50 messages (oldest first).",
            move |tenant: TenantContext, principal: Principal, input: ThreadId| {
                let service = get.clone();
                async move {
                    let viewer = Viewer::Staff(&principal);
                    let request = PageInput::new(Some(50), None, None).page()?;
                    let (thread, page) =
                        service.messages(&tenant, viewer, input.id, &request).await?;
                    let mut messages: Vec<MessageResponse> =
                        page.data.into_iter().map(MessageResponse::from).collect();
                    messages.reverse();
                    Ok::<_, Error>(ThreadWithMessages {
                        thread: ThreadResponse::new(thread, true),
                        messages,
                    })
                }
            },
        )
        .requires(READ)
        .read_only(),
        Tool::new(
            "reply",
            "Send a message to the customer of a conversation in your scope; the same `client_id` never sends twice.",
            move |tenant: TenantContext, principal: Principal, input: ReplyInput| {
                let service = reply.clone();
                async move {
                    let new = NewMessage {
                        body: input.body,
                        client_id: input.client_id,
                        media_ids: Vec::new(),
                    };
                    let viewer = Viewer::Staff(&principal);
                    let (message, _) = service.send(&tenant, viewer, input.id, new).await?;
                    Ok::<_, Error>(MessageResponse::from(message))
                }
            },
        )
        .requires(REPLY),
        Tool::new(
            "suggest_reply",
            "Ask the AI for a draft answer to a conversation; it is stored for an agent to edit and send, never sent by itself.",
            move |tenant: TenantContext, principal: Principal, input: ThreadId| {
                let service = suggest.clone();
                async move {
                    let thread = service.suggest_reply(&tenant, &principal, input.id).await?;
                    Ok::<_, Error>(DraftResponse { draft_reply: thread.draft_reply })
                }
            },
        )
        .requires(REPLY),
        Tool::new(
            "list_callbacks",
            "List call-back requests in your scope (phone numbers masked); calling back needs a person at the phone, so there is no tool for it.",
            move |tenant: TenantContext, principal: Principal, input: CallbacksInput| {
                let service = callbacks.clone();
                async move {
                    let filter = CallbackFilter { status: input.status, ..CallbackFilter::default() };
                    let page = PageInput::new(input.limit, input.cursor, None).page()?;
                    let page = service.callbacks(&tenant, &principal, &filter, &page).await?;
                    Ok::<Page<CallbackResponse>, Error>(page.map(CallbackResponse::from))
                }
            },
        )
        .requires(CALL)
        .read_only(),
        Tool::new(
            "list_calls",
            "List the calls of a conversation in your scope, newest first (no phone numbers, no recordings).",
            move |tenant: TenantContext, principal: Principal, input: ThreadId| {
                let service = calls.clone();
                async move {
                    let rows = service.calls_of(&tenant, &principal, input.id).await?;
                    Ok::<Vec<CallResponse>, Error>(rows.into_iter().map(CallResponse::from).collect())
                }
            },
        )
        .requires(CALL)
        .read_only(),
    ]
}
