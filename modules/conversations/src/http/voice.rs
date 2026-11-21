//! Endpoint gọi điện (D-232): người mua (`/me/threads/{id}/voice|calls|callbacks`), nhân viên
//! (`/callbacks`, `/threads/{id}/calls`, `/voice/agent-session`, `/calls/{id}/recording`,
//! `/voice/lines/{org_id}`), công khai (`/voice/webhooks/{provider}/{line_token}`). Số điện thoại của
//! người mua không bao giờ có trong response — chỉ dạng che.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{CALL, RECORDINGS_READ, VOICE_MANAGE};
use crate::service::ConversationsService;
use crate::voice::callbacks::{Callback, CallbackFilter, NewCallback};
use crate::voice::calls::Call;
use crate::voice::hours::Range;
use crate::voice::lines::{Line, LineInput, hours_of};

/// What the customer sees before calling.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationVoiceInfo)]
pub(crate) struct VoiceInfoResponse {
    /// The shop takes calls; otherwise hide every call button.
    pub available: bool,
    /// Calling from the browser or app works.
    pub browser_calls: bool,
    /// "Call me back" works.
    pub callbacks: bool,
    /// Calls are recorded: show `recording_notice` and send `recording_ack: true`.
    pub recording: bool,
    pub recording_notice: Option<String>,
    /// When the shop calls back (local time of the shop).
    pub callback_hours: Vec<Range>,
    pub utc_offset_minutes: Option<i32>,
}

/// A call. Phone numbers are never included.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationCall)]
pub(crate) struct CallResponse {
    pub id: Uuid,
    pub thread_id: Uuid,
    pub callback_id: Option<Uuid>,
    /// `inbound` (the customer called) or `outbound` (the shop called back).
    pub direction: String,
    /// `queued`, `ringing`, `in_progress`, `completed`, `no_answer`, `busy`, `failed`, `cancelled`.
    pub status: String,
    pub agent_user_id: Option<Uuid>,
    pub answered_at: Option<Timestamp>,
    pub ended_at: Option<Timestamp>,
    pub duration_seconds: Option<i32>,
    pub recording_enabled: bool,
    /// A recording is stored; listen at `GET /calls/{id}/recording`.
    pub recorded: bool,
    pub recording_expires_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

impl From<Call> for CallResponse {
    fn from(row: Call) -> Self {
        CallResponse {
            id: row.id,
            thread_id: row.thread_id,
            callback_id: row.callback_id,
            direction: row.direction,
            status: row.status,
            agent_user_id: row.agent_user_id,
            answered_at: row.answered_at,
            ended_at: row.ended_at,
            duration_seconds: row.duration_seconds,
            recording_enabled: row.recording_enabled,
            recorded: row.recording_ref.is_some(),
            recording_expires_at: row.recording_expires_at,
            created_at: row.created_at,
        }
    }
}

/// A call just started from the browser, with the token for the provider's WebRTC SDK.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = StartedConversationCall)]
pub(crate) struct StartedCallResponse {
    pub call: CallResponse,
    /// Short-lived token for the voice provider's SDK; do not store it.
    pub token: String,
    pub expires_at: Timestamp,
}

/// A WebRTC session for an agent.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationAgentSession)]
pub(crate) struct SessionResponse {
    pub token: String,
    pub expires_at: Timestamp,
}

/// A call-back request. The phone number is masked (`090•••567`).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationCallback)]
pub(crate) struct CallbackResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    pub thread_id: Uuid,
    pub client_id: String,
    #[schema(example = "+84•••567")]
    pub phone_masked: String,
    pub window_start: Timestamp,
    pub window_end: Timestamp,
    /// `open`, `calling`, `done`, `cancelled` or `expired`.
    pub status: String,
    pub assigned_user_id: Option<Uuid>,
    pub attempts: i32,
    pub last_call_id: Option<Uuid>,
    pub created_at: Timestamp,
}

impl From<Callback> for CallbackResponse {
    fn from(row: Callback) -> Self {
        CallbackResponse {
            id: row.id,
            org_id: row.org_id,
            thread_id: row.thread_id,
            client_id: row.client_id,
            phone_masked: row.phone_masked,
            window_start: row.window_start,
            window_end: row.window_end,
            status: row.status,
            assigned_user_id: row.assigned_user_id,
            attempts: row.attempts,
            last_call_id: row.last_call_id,
            created_at: row.created_at,
        }
    }
}

/// The phone line of a unit.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationVoiceLine)]
pub(crate) struct LineResponse {
    pub org_id: Uuid,
    pub provider: String,
    pub hotline: String,
    pub recording_enabled: bool,
    pub callback_hours: Vec<Range>,
    pub utc_offset_minutes: i32,
    pub active: bool,
    /// Path the voice provider must call (`POST`, under `/api/v1/conversations`); shown only when a
    /// new token was made — keep it, it is not shown again.
    pub webhook_path: Option<String>,
    pub updated_at: Timestamp,
}

impl LineResponse {
    fn new(line: Line, token: Option<String>) -> Self {
        let hours = hours_of(&line);
        LineResponse {
            webhook_path: token.map(|token| format!("/voice/webhooks/{}/{token}", line.provider)),
            org_id: line.org_id,
            provider: line.provider,
            hotline: line.hotline,
            recording_enabled: line.recording_enabled,
            callback_hours: hours,
            utc_offset_minutes: line.utc_offset_minutes,
            active: line.active,
            updated_at: line.updated_at,
        }
    }
}

/// Get what is needed before calling the shop.
///
/// Any signed-in account, on one of your conversations. When `available` is false, hide every call
/// button.
#[utoipa::path(
    get,
    path = "/me/threads/{id}/voice",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_my_conversation_voice",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "Call options of the shop.", body = Data<VoiceInfoResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "You have no conversation with this id; code `conversations.thread_not_found`.", body = Problem),
    )
)]
pub(crate) async fn voice_info(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<VoiceInfoResponse>, Error> {
    let info = service.voice_info(&tenant, principal.user_id(), id).await?;
    Ok(Data {
        data: VoiceInfoResponse {
            available: info.available,
            browser_calls: info.browser_calls,
            callbacks: info.callbacks,
            recording: info.recording,
            recording_notice: info.recording_notice,
            callback_hours: info.line.as_ref().map(hours_of).unwrap_or_default(),
            utc_offset_minutes: info.line.as_ref().map(|line| line.utc_offset_minutes),
        },
    })
}

/// Confirmation that the customer knows the call is recorded.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[schema(as = StartConversationCallRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartCallRequest {
    /// Required (`true`) when `GET …/voice` says `recording`.
    #[serde(default)]
    recording_ack: bool,
}

/// Call the shop from the browser.
///
/// Any signed-in account, on one of your conversations. Returns a short-lived token for the voice
/// provider's WebRTC SDK; the call goes to the shop's hotline and shows in the conversation when it
/// ends. Counts towards `AUTOXIDE_CONVERSATIONS_CALLS_PER_DAY`.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/calls",
    extensions(("x-permissions" = json!([]))),
    operation_id = "start_my_conversation_call",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = StartCallRequest,
    responses(
        (status = 201, description = "Call started.", body = Data<StartedCallResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "The shop blocked you (`conversations.blocked`) or asks for a verified account (`conversations.verification_required`).", body = Problem),
        (status = 404, description = "No such conversation (`conversations.thread_not_found`), or the shop takes no calls (`conversations.voice_unavailable`).", body = Problem),
        (status = 409, description = "The conversation is closed; code `conversations.thread_closed`.", body = Problem),
        (status = 422, description = "The call is recorded and `recording_ack` is missing; code `conversations.recording_notice_required`.", body = Problem),
        (status = 429, description = "Too many calls today; code `conversations.calls_too_many`.", body = Problem),
        (status = 503, description = "The voice provider is not answering; code `voice.provider_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn start_call(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<StartCallRequest>,
) -> Result<Created<StartedCallResponse>, Error> {
    let (call, session) =
        service.start_call(&tenant, principal.user_id(), id, input.recording_ack).await?;
    Ok(Created::new(
        format!("/conversations/me/threads/{id}"),
        StartedCallResponse {
            call: call.into(),
            token: session.token,
            expires_at: session.expires_at,
        },
    ))
}

/// When to call the customer back.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = RequestConversationCallbackRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallbackRequest {
    /// An id your app makes up (1 to 100 characters): sending again returns the same request.
    client_id: String,
    /// Start of the window, within the shop's call-back hours.
    window_start: Timestamp,
    /// End of the window, the same day.
    window_end: Timestamp,
    /// `account`: the verified phone number of your account.
    #[schema(example = "account")]
    phone_source: String,
    /// Required (`true`) when calls are recorded.
    #[serde(default)]
    recording_ack: bool,
}

/// Ask the shop to call you back.
///
/// Any signed-in account with a verified phone number. One open request per conversation; sending
/// again with the same `client_id` returns it (200). The full number is never shown to staff.
#[utoipa::path(
    post,
    path = "/me/threads/{id}/callbacks",
    extensions(("x-permissions" = json!([]))),
    operation_id = "request_my_conversation_callback",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    request_body = CallbackRequest,
    responses(
        (status = 201, description = "Requested.", body = Data<CallbackResponse>),
        (status = 200, description = "A request with this `client_id` exists; this is it.", body = Data<CallbackResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Your account has no verified phone number; code `conversations.phone_unverified`.", body = Problem),
        (status = 404, description = "No such conversation (`conversations.thread_not_found`), or the shop takes no calls (`conversations.voice_unavailable`).", body = Problem),
        (status = 409, description = "You already have an open request (`conversations.callback_pending`), or the conversation is closed (`conversations.thread_closed`).", body = Problem),
        (status = 422, description = "The window is outside the call-back hours (`conversations.callback_window_invalid`), `recording_ack` is missing (`conversations.recording_notice_required`), or a value is invalid (see `errors`).", body = Problem),
        (status = 429, description = "Too many calls today; code `conversations.calls_too_many`.", body = Problem),
    )
)]
pub(crate) async fn request_callback(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<CallbackRequest>,
) -> Result<Response, Error> {
    let new = NewCallback {
        client_id: input.client_id,
        window_start: input.window_start,
        window_end: input.window_end,
        phone_source: input.phone_source,
        recording_ack: input.recording_ack,
    };
    let (row, created) = service.request_callback(&tenant, principal.user_id(), id, new).await?;
    let data = CallbackResponse::from(row);
    Ok(if created {
        Created::new(format!("/conversations/me/threads/{id}"), data).into_response()
    } else {
        Data { data }.into_response()
    })
}

/// Cancel your call-back request.
///
/// Any signed-in account. A request the shop is calling right now cannot be cancelled.
#[utoipa::path(
    delete,
    path = "/me/threads/{id}/callbacks/{callback_id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "cancel_my_conversation_callback",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the conversation."),
        ("callback_id" = Uuid, Path, description = "Id of the request."),
    ),
    responses(
        (status = 200, description = "Cancelled (or already finished).", body = Data<CallbackResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No such conversation or request of yours; code `conversations.thread_not_found` or `conversations.callback_not_found`.", body = Problem),
        (status = 409, description = "The shop is calling you now; code `conversations.callback_taken`.", body = Problem),
    )
)]
pub(crate) async fn cancel_callback(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path((id, callback_id)): Path<(Uuid, Uuid)>,
) -> Result<Data<CallbackResponse>, Error> {
    let row = service.cancel_callback(&tenant, principal.user_id(), id, callback_id).await?;
    Ok(Data { data: row.into() })
}

/// Filters of the call-back list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct CallbackQuery {
    /// `open`, `calling`, `done`, `cancelled` or `expired`.
    status: Option<String>,
    /// Only requests of this unit.
    org_id: Option<Uuid>,
    /// Only windows ending at or after this time.
    from: Option<Timestamp>,
    /// Only windows starting at or before this time.
    to: Option<Timestamp>,
}

/// List call-back requests in your scope.
///
/// Requires `conversations.call`. Phone numbers are masked. Newest first unless `order=asc`.
#[utoipa::path(
    get,
    path = "/callbacks",
    extensions(("x-permissions" = json!(["conversations.call"]))),
    operation_id = "list_conversation_callbacks",
    security(("bearer" = [])),
    params(CallbackQuery, PageQuery),
    responses(
        (status = 200, description = "One page of requests.", body = Page<CallbackResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.call`; code `permission_denied`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_callbacks(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<CallbackQuery>,
    Query(page): Query<PageQuery>,
) -> Result<Page<CallbackResponse>, Error> {
    principal.require(&CALL)?;
    let filter = CallbackFilter {
        status: query.status,
        org_id: query.org_id,
        from: query.from,
        to: query.to,
    };
    let page =
        service.callbacks(&tenant, &principal, &filter, &PageRequest::try_from(page)?).await?;
    Ok(page.map(CallbackResponse::from))
}

/// Call a customer back.
///
/// Requires `conversations.call` at the unit of the request, and an `Idempotency-Key` header. The
/// customer sees the shop's hotline; the agent answers on their WebRTC session. Two agents pressing
/// at once make one call; the other gets `conversations.callback_taken`.
#[utoipa::path(
    post,
    path = "/callbacks/{id}/call",
    extensions(("x-permissions" = json!(["conversations.call"])), ("x-idempotency" = json!("required"))),
    operation_id = "call_conversation_callback",
    security(("bearer" = [])),
    params(
        ("id" = Uuid, Path, description = "Id of the request."),
        ("Idempotency-Key" = String, Header, description = "Required: a key you make up; sending again returns the same call."),
    ),
    responses(
        (status = 201, description = "Calling.", body = Data<CallResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.call`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such request in your scope (`conversations.callback_not_found`), or the unit takes no calls (`conversations.voice_unavailable`).", body = Problem),
        (status = 409, description = "Another agent is calling; code `conversations.callback_taken`.", body = Problem),
        (status = 503, description = "The voice provider is not answering; the request is open again. Code `voice.provider_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn call_back(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Created<CallResponse>, Error> {
    principal.require(&CALL)?;
    if headers.get("idempotency-key").is_none() {
        return Err(Error::validation(vec![autoxide_kernel::error::FieldError::new(
            "Idempotency-Key",
            "required",
            "is required",
        )]));
    }
    let call = service.call_back(&tenant, &principal, id).await?;
    Ok(Created::new(format!("/conversations/threads/{}/calls", call.thread_id), call.into()))
}

/// Close a call-back request.
///
/// Requires `conversations.call`. Use it when the customer was reached another way.
#[utoipa::path(
    post,
    path = "/callbacks/{id}/close",
    extensions(("x-permissions" = json!(["conversations.call"]))),
    operation_id = "close_conversation_callback",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the request.")),
    responses(
        (status = 200, description = "Closed (or already finished).", body = Data<CallbackResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.call`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such request in your scope; code `conversations.callback_not_found`.", body = Problem),
        (status = 409, description = "An agent is calling now; code `conversations.callback_taken`.", body = Problem),
    )
)]
pub(crate) async fn close_callback(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CallbackResponse>, Error> {
    principal.require(&CALL)?;
    let row = service.close_callback(&tenant, &principal, id).await?;
    Ok(Data { data: row.into() })
}

/// List the calls of a conversation.
///
/// Requires `conversations.call` at the unit of the conversation. Newest first; no phone numbers.
#[utoipa::path(
    get,
    path = "/threads/{id}/calls",
    extensions(("x-permissions" = json!(["conversations.call"]))),
    operation_id = "list_conversation_calls",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the conversation.")),
    responses(
        (status = 200, description = "The calls.", body = Data<Vec<CallResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.call`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No conversation with this id in your scope; code `conversations.thread_not_found`.", body = Problem),
    )
)]
pub(crate) async fn list_calls(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<CallResponse>>, Error> {
    principal.require(&CALL)?;
    let calls = service.calls_of(&tenant, &principal, id).await?;
    Ok(Data { data: calls.into_iter().map(CallResponse::from).collect() })
}

/// The unit whose line the agent works on.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SessionQuery {
    /// Unit of the phone line; omitted: the shop (root unit).
    org_id: Option<Uuid>,
}

/// Get a WebRTC session to answer calls.
///
/// Requires `conversations.call` at the unit. Returns a short-lived token for the voice provider's
/// SDK; calls to the hotline ring on it.
#[utoipa::path(
    get,
    path = "/voice/agent-session",
    extensions(("x-permissions" = json!(["conversations.call"]))),
    operation_id = "get_conversation_agent_session",
    security(("bearer" = [])),
    params(SessionQuery),
    responses(
        (status = 200, description = "The session.", body = Data<SessionResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.call` there; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "The unit takes no calls; code `conversations.voice_unavailable`.", body = Problem),
        (status = 503, description = "The voice provider is not answering; code `voice.provider_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn agent_session(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<SessionQuery>,
) -> Result<Data<SessionResponse>, Error> {
    principal.require(&CALL)?;
    let org = query.org_id.unwrap_or_else(|| tenant.id().as_uuid());
    let session = service.agent_session(&tenant, &principal, org).await?;
    Ok(Data { data: SessionResponse { token: session.token, expires_at: session.expires_at } })
}

/// Listen to a call recording.
///
/// Requires `conversations.recordings.read` at the unit of the call. Every listen is recorded
/// (event `conversations.recording_opened`).
#[utoipa::path(
    get,
    path = "/calls/{id}/recording",
    extensions(("x-permissions" = json!(["conversations.recordings.read"]))),
    operation_id = "get_conversation_call_recording",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the call.")),
    responses(
        (status = 200, description = "The recording, as audio.", content_type = "application/octet-stream", body = Vec<u8>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.recordings.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such call in your scope (`conversations.call_not_found`) or no recording (`conversations.recording_not_found`).", body = Problem),
    )
)]
pub(crate) async fn get_recording(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Response, Error> {
    principal.require(&RECORDINGS_READ)?;
    let file = service.open_recording(&tenant, &principal, id).await?;
    let mut response = Body::from_stream(file.content).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file.size));
    Ok(response)
}

/// Delete a call recording.
///
/// Requires `conversations.voice.manage` at the unit of the call — for example when the customer
/// asks. Deleting again changes nothing.
#[utoipa::path(
    delete,
    path = "/calls/{id}/recording",
    extensions(("x-permissions" = json!(["conversations.voice.manage"]))),
    operation_id = "delete_conversation_call_recording",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the call.")),
    responses(
        (status = 200, description = "Deleted; the call stays.", body = Data<CallResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.voice.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such call in your scope; code `conversations.call_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_recording(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<CallResponse>, Error> {
    principal.require(&VOICE_MANAGE)?;
    let call = service.delete_recording(&tenant, &principal, id).await?;
    Ok(Data { data: call.into() })
}

/// Get the phone line of a unit.
///
/// Requires `conversations.voice.manage` at the unit.
#[utoipa::path(
    get,
    path = "/voice/lines/{org_id}",
    extensions(("x-permissions" = json!(["conversations.voice.manage"]))),
    operation_id = "get_conversation_voice_line",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Id of the unit.")),
    responses(
        (status = 200, description = "The line.", body = Data<LineResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.voice.manage` there; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "The unit has no line; code `conversations.voice_unavailable`.", body = Problem),
    )
)]
pub(crate) async fn get_line(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Data<LineResponse>, Error> {
    principal.require(&VOICE_MANAGE)?;
    let line = service
        .get_line(&tenant, &principal, org_id)
        .await?
        .ok_or_else(crate::voice::lines::voice_unavailable)?;
    Ok(Data { data: LineResponse::new(line, None) })
}

/// The phone line of a unit.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetConversationVoiceLineRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct LineRequest {
    /// A voice provider installed on this server (`provider()` of its connector, e.g. `stringee`).
    provider: String,
    /// Public number of the shop; customers see it when called back.
    hotline: String,
    /// Record calls (off by default). Only when the provider plays a recording notice.
    #[serde(default)]
    recording_enabled: bool,
    /// Call-back hours (default 08:00–20:00 every day).
    callback_hours: Option<Vec<Range>>,
    /// Minutes from UTC of the hours (default 420, Vietnam).
    utc_offset_minutes: Option<i32>,
    #[serde(default = "yes")]
    active: bool,
    /// Make a new webhook token (the old one stops working).
    #[serde(default)]
    rotate_token: bool,
}

fn yes() -> bool {
    true
}

/// Set the phone line of a unit.
///
/// Requires `conversations.voice.manage` at the unit. The first time (and with `rotate_token`) the
/// response carries `webhook_path` to give the voice provider — it is not shown again.
#[utoipa::path(
    put,
    path = "/voice/lines/{org_id}",
    extensions(("x-permissions" = json!(["conversations.voice.manage"]))),
    operation_id = "set_conversation_voice_line",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "Id of the unit.")),
    request_body = LineRequest,
    responses(
        (status = 200, description = "The line.", body = Data<LineResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `conversations.voice.manage` there; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 422, description = "A value is invalid (see `errors`), or recording cannot be switched on with this provider (`conversations.recording_unsupported`).", body = Problem),
    )
)]
pub(crate) async fn set_line(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    ValidJson(input): ValidJson<LineRequest>,
) -> Result<Data<LineResponse>, Error> {
    principal.require(&VOICE_MANAGE)?;
    let input = LineInput {
        provider: input.provider,
        hotline: input.hotline,
        recording_enabled: input.recording_enabled,
        callback_hours: input.callback_hours,
        utc_offset_minutes: input.utc_offset_minutes,
        active: input.active,
        rotate_token: input.rotate_token,
    };
    let (line, token) = service.put_line(&tenant, &principal, org_id, input).await?;
    Ok(Data { data: LineResponse::new(line, token) })
}

/// How many notifications were stored.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = ConversationVoiceWebhookReceipt)]
pub(crate) struct WebhookReceipt {
    /// New notifications stored (repeats are not counted).
    pub stored: usize,
}

/// Receive a call status notification from the voice provider.
///
/// Public: the provider calls the `webhook_path` given when the line was set up. The signature is
/// checked on the raw body; a wrong signature stores nothing. Repeated or late notifications are
/// applied once and never move a call backwards.
#[utoipa::path(
    post,
    path = "/voice/webhooks/{provider}/{line_token}",
    operation_id = "receive_conversation_voice_webhook",
    extensions(("x-public" = json!(true))),
    security(()),
    params(
        ("provider" = String, Path, description = "Name of the voice provider."),
        ("line_token" = String, Path, description = "Token of the phone line (from `webhook_path`)."),
    ),
    request_body(content = String, description = "Body as the provider sends it.", content_type = "application/json"),
    responses(
        (status = 200, description = "Stored; applied shortly.", body = Data<WebhookReceipt>),
        (status = 401, description = "The signature is not valid; nothing was stored. Code `voice.signature_invalid`.", body = Problem),
        (status = 404, description = "No line of this provider has this token; code `conversations.voice_unavailable`.", body = Problem),
        (status = 413, description = "The body is larger than `AUTOXIDE_CONVERSATIONS_VOICE_WEBHOOK_MAX_BYTES`.", body = Problem),
    )
)]
pub(crate) async fn receive_webhook(
    State(service): State<Arc<ConversationsService>>,
    tenant: TenantContext,
    Path((provider, line_token)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Data<WebhookReceipt>), Error> {
    let stored =
        service.receive_voice_webhook(&tenant, &provider, &line_token, &headers, &body).await?;
    Ok((StatusCode::OK, Data { data: WebhookReceipt { stored } }))
}
