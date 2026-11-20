//! `GET /api/v1/kernel/events/ws` (bước 69 P3, D-20261119-step69-ws): event thời gian thực qua
//! WebSocket, cho app cần hai chiều (chat, vị trí người giao hàng). Dùng chung [`StreamHub`] và bộ
//! lọc quyền của SSE (D-378): cùng event, cùng quyền, cùng cách nối lại (`last_event_id`), cùng
//! giới hạn kết nối — một phiên WebSocket là một kết nối dù đổi đăng ký bao nhiêu lần.
//!
//! Máy khách chỉ gửi được LỆNH ĐIỀU KHIỂN: đăng ký / huỷ đăng ký event, báo đã nhận, vào / rời một
//! chủ đề và báo "đang gõ" (tín hiệu tạm, không lưu — [`crate::event::signals`]). Ghi dữ liệu vẫn
//! qua REST: mỗi việc một cách làm.
//!
//! Xác thực như REST lúc bắt tay; sau đó cứ `AUTOXIDE_EVENTS_WS_RECHECK_SECONDS` giây xét lại —
//! token hết hạn, tài khoản bị khoá thì máy chủ đóng với mã 4401. Mỗi 30 phút (hết đời kết nối
//! của hub) quyền được xét lại như khi nối mới.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{CloseFrame, Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::Principal;
use crate::error::{Error, ErrorKind};
use crate::event::StreamHub;
use crate::event::signals::SignalTopicsSlot;
use crate::event::stream::{Frame, StreamedEvent, Ticket, disabled, parse_last_event_id};
use crate::tenancy::TenantContext;

/// Mã đóng: thông tin xác thực không còn dùng được (token hết hạn, tài khoản bị khoá…).
const CLOSE_UNAUTHENTICATED: u16 = 4401;
/// Mã đóng: không còn quyền xem event nào đã đăng ký.
const CLOSE_FORBIDDEN: u16 = 4403;
/// Mã đóng chuẩn (RFC 6455): máy chủ khởi động lại / đang tắt — nối lại, máy chủ khác trả lời.
const CLOSE_RESTART: u16 = 1012;
/// Thân một thông điệp của máy khách tối đa chừng này byte.
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
/// Một kết nối báo "đang gõ" ở một chủ đề tối đa một lần trong khoảng này; lần dày hơn bị bỏ.
const TYPING_EVERY: Duration = Duration::from_secs(2);
/// Nhớ event đã gửi chừng này để không gửi lại khi đổi đăng ký.
const REMEMBER_SENT: Duration = Duration::from_secs(60);
/// Một kết nối vào tối đa chừng này chủ đề.
const MAX_TOPICS: usize = 20;

/// Trạng thái chung của endpoint.
pub(crate) struct WsState {
    hub: StreamHub,
    topics: SignalTopicsSlot,
    /// Tín hiệu tạm giữa các kết nối của tiến trình này.
    signals: broadcast::Sender<Arc<Signal>>,
    /// Ai đang ở chủ đề nào: (khách, chủ đề) → kết nối → người dùng.
    present: Mutex<HashMap<(Uuid, String), HashMap<Uuid, Uuid>>>,
}

pub(crate) fn routes(hub: StreamHub, topics: SignalTopicsSlot) -> OpenApiRouter {
    let (signals, _) = broadcast::channel(1024);
    let state = Arc::new(WsState { hub, topics, signals, present: Mutex::new(HashMap::new()) });
    OpenApiRouter::new().routes(routes!(events_ws)).with_state(state)
}

/// What to receive first.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct WsQuery {
    /// Event names, comma-separated: `module.event`, or `module.*` for every streamable event of
    /// the module you may see. Change them later with `subscribe` / `unsubscribe` messages.
    #[param(example = "content.entry_published,comments.*")]
    names: String,
    /// Resume after this event id: the `id` of the last event you received.
    last_event_id: Option<String>,
}

/// Receive events and send live signals over a WebSocket.
///
/// The two-way companion of `GET /api/v1/kernel/events/stream` (Server-Sent Events, still the
/// choice for browsers): the same events, permissions and resuming. Sign in as for REST (a bearer
/// token or an API key in `Authorization`); the handshake answers the same errors as the event
/// stream. Every message is a JSON text frame.
///
/// From the server: `{"type": "event", "event": {"id", "name", "occurred_at", "subject",
/// "actor_kind", "payload"}}`; `{"type": "reset"}` (what you missed is no longer kept: reload your
/// data over REST); `{"type": "subscribed", "names": […]}`; `{"type": "joined", "topic", "online":
/// [user ids]}`, `{"type": "left", "topic"}`; `{"type": "signal", "topic", "user_id", "signal":
/// "online" | "offline" | "typing"}`; `{"type": "pong"}`; `{"type": "error", "code", "message"}`.
///
/// From you — control messages only; writing data stays with REST: `{"type": "subscribe" |
/// "unsubscribe", "names": "a.b,c.*"}`, `{"type": "ack", "id"}` (the last event you processed),
/// `{"type": "join" | "leave" | "typing", "topic": "<kind>:<id>"}` (live signals in a topic a module
/// opens, e.g. a conversation; not stored), `{"type": "ping"}`.
///
/// Delivery is at least once: skip event ids you already have. Close codes: `4401` your
/// credentials no longer hold (token expired: sign in and reconnect), `4403` you may no longer
/// receive any event you named, `1012` the server restarts (reconnect with `last_event_id`).
/// Disabled unless the server sets `AUTOXIDE_EVENTS_STREAM=true`.
#[utoipa::path(
    get,
    path = "/events/ws",
    operation_id = "events_websocket",
    // Permission tuỳ event được nêu (`stream_permission` của từng event), như SSE.
    extensions(("x-permissions" = json!([]))),
    security(("bearer" = [])),
    params(WsQuery),
    responses(
        (status = 101, description = "Switched to the WebSocket protocol."),
        (status = 400, description = "Not a WebSocket handshake; code `bad_request`.", body = crate::response::Problem),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = crate::response::Problem),
        (status = 403, description = "You may not receive an event you named (code `permission_denied`, naming the permission), or any of them.", body = crate::response::Problem),
        (status = 404, description = "Streaming is disabled on this server; code `kernel.stream_disabled`.", body = crate::response::Problem),
        (status = 422, description = "An event does not exist or is not streamable (code `kernel.stream_unknown_event`), or `names` / `last_event_id` is invalid (see `errors`).", body = crate::response::Problem),
        (status = 429, description = "Too many streams are open (per user, per tenant or on this server); code `kernel.stream_too_many`.", body = crate::response::Problem),
        (status = 503, description = "The server is shutting down; connect again; code `shutting_down`.", body = crate::response::Problem),
    )
)]
async fn events_ws(
    State(state): State<Arc<WsState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<WsQuery>,
    parts: Parts,
    // Nhận cả lỗi bắt tay thay vì để axum từ chối ngay: client biết trước máy chủ tắt stream (404),
    // thiếu đăng nhập (401) hay sai `names` (422) — cùng thứ tự lỗi với SSE — rồi mới tới lỗi bắt tay.
    upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Result<Response, Error> {
    if !state.hub.enabled() {
        return Err(disabled());
    }
    let names = patterns(&query.names);
    let filter = state.hub.filter(&tenant, &principal, &join(&names))?;
    let last = query
        .last_event_id
        .as_deref()
        .filter(|raw| !raw.trim().is_empty())
        .map(parse_last_event_id)
        .transpose()?;
    let upgrade = match upgrade {
        Ok(upgrade) => upgrade,
        Err(rejection) => return Ok(rejection.into_response()),
    };
    // Giữ chỗ trước khi đổi giao thức: vượt giới hạn thì trả 429 như SSE.
    let ticket = state.hub.session(&tenant, principal.user_id())?;
    let receiver = state.hub.connect(&tenant, filter, last, None);
    let session = Session {
        id: Uuid::now_v7(),
        signals: state.signals.subscribe(),
        state,
        tenant,
        principal,
        parts,
        names,
        receiver: Some(receiver),
        last,
        // Event cuối máy khách báo đã có: luồng mới sau khi đổi đăng ký không gửi lại nó.
        sent: last.map(|id| (id, Instant::now())).into_iter().collect(),
        topics: HashMap::new(),
        _ticket: ticket,
    };
    Ok(upgrade.max_message_size(MAX_MESSAGE_BYTES).on_upgrade(move |socket| session.run(socket)))
}

/// Tên event từ chuỗi `a,b,c`, bỏ trống và trùng.
fn patterns(names: &str) -> BTreeSet<String> {
    names.split(',').map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect()
}

fn join(names: &BTreeSet<String>) -> String {
    names.iter().map(String::as_str).collect::<Vec<_>>().join(",")
}

/// Một tín hiệu tạm.
#[derive(Debug, Clone, Serialize)]
struct Signal {
    #[serde(skip)]
    tenant: Uuid,
    /// Kết nối gửi (không gửi lại cho chính nó).
    #[serde(skip)]
    from: Uuid,
    topic: String,
    user_id: Uuid,
    signal: &'static str,
}

/// Thông điệp máy khách gửi.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Subscribe { names: String },
    Unsubscribe { names: String },
    Ack { id: String },
    Join { topic: String },
    Leave { topic: String },
    Typing { topic: String },
    Ping,
}

/// Thông điệp máy chủ gửi.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Reply<'a> {
    Event { event: &'a StreamedEvent },
    Reset,
    Subscribed { names: Vec<&'a str> },
    Joined { topic: &'a str, online: Vec<Uuid> },
    Left { topic: &'a str },
    Signal(&'a Signal),
    Pong,
    Error { code: &'a str, message: String },
}

/// Một phiên WebSocket.
struct Session {
    id: Uuid,
    state: Arc<WsState>,
    tenant: TenantContext,
    principal: Principal,
    /// Phần request bắt tay: xét lại thông tin xác thực bằng đúng đường của REST.
    parts: Parts,
    names: BTreeSet<String>,
    /// Luồng khung từ hub; `None` khi không đăng ký event nào.
    receiver: Option<mpsc::Receiver<Frame>>,
    /// Event cuối đã gửi (hay máy khách báo đã nhận): nối lại từ đây khi đổi đăng ký.
    last: Option<Uuid>,
    signals: broadcast::Receiver<Arc<Signal>>,
    /// Event đã gửi gần đây → lúc gửi: luồng mới sau khi đổi đăng ký đọc bù lùi lại vài giây, không
    /// gửi lại event phiên đã gửi.
    sent: HashMap<Uuid, Instant>,
    /// Chủ đề đã vào → lần báo "đang gõ" gần nhất.
    topics: HashMap<String, Option<Instant>>,
    _ticket: Ticket,
}

/// Phiên kết thúc thế nào.
enum End {
    /// Máy khách đóng hay mất kết nối.
    Gone,
    /// Máy chủ đóng với mã và lý do.
    Close(u16, String),
}

impl Session {
    async fn run(mut self, mut socket: WebSocket) {
        let end = self.serve(&mut socket).await;
        self.leave_all();
        if let End::Close(code, reason) = end {
            let frame = CloseFrame { code, reason: Utf8Bytes::from(reason) };
            let _ = socket.send(Message::Close(Some(frame))).await;
        }
    }

    async fn serve(&mut self, socket: &mut WebSocket) -> End {
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        let subscribed = text(&Reply::Subscribed { names });
        if socket.send(subscribed).await.is_err() {
            return End::Gone;
        }
        let recheck = self.state.hub.ws_recheck();
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + recheck, recheck);
        loop {
            tokio::select! {
                frame = next_frame(&mut self.receiver) => {
                    let message = match frame {
                        Some(Frame::Event(event)) => {
                            if !self.fresh(event.id) {
                                continue;
                            }
                            self.last = Some(event.id);
                            text(&Reply::Event { event: &event })
                        }
                        Some(Frame::Reset) => text(&Reply::Reset),
                        // Hub đóng luồng: hết đời kết nối (xét lại quyền rồi nối tiếp) hay tắt.
                        None => match self.reconnect().await {
                            Ok(()) => continue,
                            Err(end) => return end,
                        },
                    };
                    if socket.send(message).await.is_err() {
                        return End::Gone;
                    }
                }
                received = socket.recv() => {
                    let message = match received {
                        Some(Ok(Message::Text(body))) => body,
                        Some(Ok(Message::Close(_)) | Err(_)) | None => return End::Gone,
                        // Ping / pong do thư viện trả lời; khung nhị phân không có nghĩa: bỏ qua.
                        Some(Ok(_)) => continue,
                    };
                    let replies = self.command(message.as_str()).await;
                    for reply in replies {
                        if socket.send(reply).await.is_err() {
                            return End::Gone;
                        }
                    }
                }
                signal = self.signals.recv() => {
                    let Ok(signal) = signal else { continue };
                    if signal.tenant != self.tenant.id().as_uuid()
                        || signal.from == self.id
                        || !self.topics.contains_key(&signal.topic)
                    {
                        continue;
                    }
                    if socket.send(text(&Reply::Signal(&signal))).await.is_err() {
                        return End::Gone;
                    }
                }
                _ = tick.tick() => {
                    if let Err(error) = self.authenticate().await {
                        return End::Close(CLOSE_UNAUTHENTICATED, error.code().to_owned());
                    }
                }
            }
        }
    }

    /// `id` chưa gửi trong phiên (ghi nhận là đã gửi). Chỉ nhớ một phút: đủ cho khoảng đọc bù.
    fn fresh(&mut self, id: Uuid) -> bool {
        if self.sent.len() > 1000 {
            self.sent.retain(|_, at| at.elapsed() < REMEMBER_SENT);
        }
        self.sent.insert(id, Instant::now()).is_none()
    }

    /// Xét lại thông tin xác thực của lần bắt tay (token còn hạn, tài khoản còn dùng được).
    async fn authenticate(&mut self) -> Result<(), Error> {
        let mut parts = self.parts.clone();
        parts.extensions.remove::<crate::tenancy::request::Authenticated>();
        self.principal = Principal::from_request_parts(&mut parts, &()).await?;
        Ok(())
    }

    /// Nối lại luồng của hub sau khi nó đóng, với quyền xét lại.
    async fn reconnect(&mut self) -> Result<(), End> {
        self.receiver = None;
        if self.state.hub.is_closed() {
            return Err(End::Close(CLOSE_RESTART, "shutting_down".to_owned()));
        }
        if let Err(error) = self.authenticate().await {
            return Err(End::Close(CLOSE_UNAUTHENTICATED, error.code().to_owned()));
        }
        match self.state.hub.filter(&self.tenant, &self.principal, &join(&self.names)) {
            Ok(filter) => {
                self.receiver = Some(self.state.hub.connect(&self.tenant, filter, self.last, None));
                Ok(())
            }
            Err(error) => Err(End::Close(CLOSE_FORBIDDEN, error.code().to_owned())),
        }
    }

    /// Làm một lệnh của máy khách; trả các thông điệp cần gửi lại.
    async fn command(&mut self, body: &str) -> Vec<Message> {
        let command = match serde_json::from_str::<Command>(body) {
            Ok(command) => command,
            Err(_) => return vec![error(&unknown_command())],
        };
        match command {
            Command::Subscribe { names } | Command::Unsubscribe { names }
                if patterns(&names).is_empty() =>
            {
                vec![error(&Error::validation(vec![crate::error::FieldError::new(
                    "names",
                    "required",
                    "is required",
                )]))]
            }
            Command::Subscribe { names } => {
                let mut wanted = self.names.clone();
                wanted.extend(patterns(&names));
                self.resubscribe(wanted)
            }
            Command::Unsubscribe { names } => {
                let mut wanted = self.names.clone();
                for name in patterns(&names) {
                    wanted.remove(&name);
                }
                self.resubscribe(wanted)
            }
            Command::Ack { id } => match parse_last_event_id(&id) {
                Ok(id) => {
                    self.last = Some(id);
                    Vec::new()
                }
                Err(problem) => vec![error(&problem)],
            },
            Command::Join { topic } => self.join(topic).await,
            Command::Leave { topic } => {
                if self.topics.remove(&topic).is_some() {
                    self.announce(&topic, "offline");
                }
                vec![text(&Reply::Left { topic: &topic })]
            }
            Command::Typing { topic } => {
                match self.topics.get_mut(&topic) {
                    None => return vec![error(&not_joined(&topic))],
                    Some(last) if last.is_some_and(|at| at.elapsed() < TYPING_EVERY) => {}
                    Some(last) => {
                        *last = Some(Instant::now());
                        self.announce(&topic, "typing");
                    }
                }
                Vec::new()
            }
            Command::Ping => vec![text(&Reply::Pong)],
        }
    }

    /// Đổi tập event đăng ký: kiểm quyền với tập mới (sai thì giữ tập cũ), rồi nối lại luồng từ
    /// event cuối đã gửi — không lọt event nào giữa hai luồng.
    fn resubscribe(&mut self, wanted: BTreeSet<String>) -> Vec<Message> {
        if wanted.is_empty() {
            self.names = wanted;
            self.receiver = None;
        } else {
            match self.state.hub.filter(&self.tenant, &self.principal, &join(&wanted)) {
                Ok(filter) => {
                    self.names = wanted;
                    self.receiver =
                        Some(self.state.hub.connect(&self.tenant, filter, self.last, None));
                }
                Err(problem) => return vec![error(&problem)],
            }
        }
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        vec![text(&Reply::Subscribed { names })]
    }

    /// Vào chủ đề `topic` (`<loại>:<id>`) nếu module của loại đó cho phép.
    async fn join(&mut self, topic: String) -> Vec<Message> {
        if self.topics.contains_key(&topic) {
            return vec![self.joined(&topic)];
        }
        if self.topics.len() >= MAX_TOPICS {
            return vec![error(&too_many_topics())];
        }
        let allowed = match topic.split_once(':').and_then(|(kind, id)| {
            self.state.topics.find(kind).map(|topics| (topics, id.to_owned()))
        }) {
            None => Err(unknown_topic(&topic)),
            Some((topics, id)) => match topics.allows(&self.tenant, &self.principal, &id).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(topic_denied(&topic)),
                Err(problem) => Err(problem),
            },
        };
        if let Err(problem) = allowed {
            return vec![error(&problem)];
        }
        self.topics.insert(topic.clone(), None);
        self.announce(&topic, "online");
        vec![self.joined(&topic)]
    }

    fn joined(&self, topic: &str) -> Message {
        let key = (self.tenant.id().as_uuid(), topic.to_owned());
        let present = lock(&self.state.present);
        let mut online: Vec<Uuid> =
            present.get(&key).map(|users| users.values().copied().collect()).unwrap_or_default();
        online.sort();
        online.dedup();
        text(&Reply::Joined { topic, online })
    }

    /// Gửi tín hiệu `signal` của người dùng này ở `topic` cho các kết nối khác; cập nhật sổ có mặt.
    fn announce(&self, topic: &str, signal: &'static str) {
        let key = (self.tenant.id().as_uuid(), topic.to_owned());
        {
            let mut present = lock(&self.state.present);
            match signal {
                "online" => {
                    present.entry(key).or_default().insert(self.id, self.principal.user_id());
                }
                "offline" => {
                    if let Some(users) = present.get_mut(&key) {
                        users.remove(&self.id);
                        if users.is_empty() {
                            present.remove(&key);
                        }
                    }
                }
                _ => {}
            }
        }
        let signal = Signal {
            tenant: self.tenant.id().as_uuid(),
            from: self.id,
            topic: topic.to_owned(),
            user_id: self.principal.user_id(),
            signal,
        };
        // Không ai nghe thì `send` báo lỗi: bỏ qua.
        let _ = self.state.signals.send(Arc::new(signal));
    }

    /// Rời mọi chủ đề khi phiên kết thúc.
    fn leave_all(&mut self) {
        let topics: Vec<String> = self.topics.drain().map(|(topic, _)| topic).collect();
        for topic in topics {
            self.announce(&topic, "offline");
        }
    }
}

/// Khung kế tiếp của luồng hub; không có luồng thì chờ mãi (nhánh khác của `select!` chạy).
async fn next_frame(receiver: &mut Option<mpsc::Receiver<Frame>>) -> Option<Frame> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

fn text(reply: &Reply<'_>) -> Message {
    Message::text(serde_json::to_string(reply).unwrap_or_else(|_| "{}".to_owned()))
}

/// Thông điệp lỗi: mã ổn định và câu tiếng Anh đã điền giá trị.
fn error(problem: &Error) -> Message {
    let mut message = problem.message();
    for field in problem.fields() {
        message.push_str(&format!(" `{}` {}.", field.field, field.message));
    }
    text(&Reply::Error { code: problem.code(), message })
}

fn unknown_command() -> Error {
    Error::new(
        ErrorKind::Validation,
        "kernel.ws_unknown_command",
        "Unknown message. Send JSON with `type`: `subscribe`, `unsubscribe`, `ack`, `join`, \
         `leave`, `typing` or `ping`. Data is written over REST, not over this socket.",
    )
}

fn unknown_topic(topic: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "kernel.ws_unknown_topic",
        "No module opens live signals for `{topic}`. Topics look like `<kind>:<id>`.",
    )
    .with_param("topic", topic)
}

fn topic_denied(topic: &str) -> Error {
    Error::forbidden("kernel.ws_topic_denied", "You may not join `{topic}`.")
        .with_param("topic", topic)
}

fn not_joined(topic: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "kernel.ws_not_joined",
        "Join `{topic}` (a `join` message) before sending signals there.",
    )
    .with_param("topic", topic)
}

fn too_many_topics() -> Error {
    Error::new(
        ErrorKind::TooManyRequests,
        "kernel.ws_too_many_topics",
        "One connection joins at most {max} topics. Leave one first.",
    )
    .with_param("max", MAX_TOPICS)
}

/// Mutex bị "đầu độc" vẫn dùng tiếp: bên trong chỉ là sổ ai đang có mặt.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
