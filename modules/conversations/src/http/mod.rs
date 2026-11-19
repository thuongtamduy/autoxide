//! Endpoint REST của module `conversations`, dưới `/api/v1/conversations`. Handler chỉ: lấy dữ liệu
//! vào → gọi service → trả envelope. Hai nhóm: nhân viên (`/threads`, theo permission ở nút), người
//! mua (`/me/…`, chỉ cuộc của mình). Thời gian thực: stream SSE của kernel
//! (`GET /api/v1/kernel/events/stream?names=conversations.*`).

mod me;
mod staff;
pub(crate) mod views;

use std::sync::Arc;

use autoxide_kernel::response::{Created, Data};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::messages::NewMessage;
use crate::service::{ConversationsService, Message};
use views::MessageResponse;

pub(crate) fn routes(service: Arc<ConversationsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(staff::list_threads))
        .routes(routes!(staff::get_thread))
        .routes(routes!(staff::list_messages, staff::reply))
        .routes(routes!(staff::read))
        .routes(routes!(staff::typing))
        .routes(routes!(staff::recall))
        .routes(routes!(staff::assign))
        .routes(routes!(staff::close))
        .routes(routes!(staff::escalate))
        .routes(routes!(staff::suggest_reply))
        .routes(routes!(staff::block, staff::unblock))
        .routes(routes!(staff::report))
        .routes(routes!(staff::list_reports))
        .routes(routes!(staff::resolve_report))
        .routes(routes!(me::list_mine, me::start))
        .routes(routes!(me::get_mine))
        .routes(routes!(me::list_my_messages, me::add_my_message))
        .routes(routes!(me::read_mine))
        .routes(routes!(me::typing_mine))
        .routes(routes!(me::recall_mine))
        .routes(routes!(me::close_mine))
        .routes(routes!(me::handoff))
        .routes(routes!(me::report_mine))
        .with_state(service)
}

/// A chat message to send.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ConversationMessageRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct MessageRequest {
    /// Plain text; may be empty when you send images.
    #[serde(default)]
    body: String,
    /// An id your app makes up for this message (1 to 100 characters): sending again with the same
    /// id does not add a second message.
    client_id: String,
    /// Images from the media library (customers: only images they uploaded); at most 5.
    #[serde(default)]
    media_ids: Vec<Uuid>,
}

impl From<MessageRequest> for NewMessage {
    fn from(input: MessageRequest) -> Self {
        NewMessage { body: input.body, client_id: input.client_id, media_ids: input.media_ids }
    }
}

/// How far you have read.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = ReadConversationRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadRequest {
    /// The last message you have read; omitted: the newest message.
    message_id: Option<Uuid>,
}

/// A report of abuse.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = ReportConversationRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReportRequest {
    /// The message you report; omitted: the whole conversation.
    message_id: Option<Uuid>,
    /// What is wrong, 1 to 1000 characters.
    reason: String,
}

/// 201 cho tin mới, 200 cho tin đã gửi trước với cùng `client_id`.
fn sent(location: String, message: Message, created: bool) -> Response {
    let data = MessageResponse::from(message);
    if created {
        Created::new(location, data).into_response()
    } else {
        Data { data }.into_response()
    }
}
