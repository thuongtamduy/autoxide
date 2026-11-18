//! `GET /api/v1/kernel/events/stream` (bước 40 P5, D-378): Server-Sent Events của các event người
//! gọi được xem. Logic ở [`StreamHub`](crate::event::StreamHub); handler chỉ lấy dữ liệu vào, gọi
//! hub và dựng response.
//!
//! Lớp timeout chung không cắt stream: nó chỉ chờ tới khi handler trả response (header); thân
//! stream chảy sau đó. Graceful shutdown đóng stream qua `StreamHub::close`.

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt as _;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::Principal;
use crate::error::Error;
use crate::event::StreamHub;
use crate::event::stream::{Frame, disabled, parse_last_event_id};
use crate::tenancy::TenantContext;

/// Gửi `: ping` mỗi chừng này: proxy không đóng kết nối "im lặng", client biết kết nối còn sống.
const PING: Duration = Duration::from_secs(15);
/// Client nối lại sau chừng này khi mất kết nối (`retry:` của SSE).
const RETRY: Duration = Duration::from_secs(3);

pub(crate) fn routes(hub: StreamHub) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(stream_events)).with_state(hub)
}

/// What to stream.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct StreamQuery {
    /// Event names, comma-separated: `module.event`, or `module.*` for every streamable event of
    /// the module you may see. The module catalog lists each event's `stream_permission`.
    #[param(example = "content.entry_published,comments.*")]
    names: String,
    /// Resume after this event id, for clients that cannot set the `Last-Event-ID` header.
    last_event_id: Option<String>,
}

/// Stream events in real time (Server-Sent Events).
///
/// Requires sign-in, and for each event the permission its module declares (see
/// `stream_permission` in `GET /api/v1/kernel/modules`): the same permission as reading the record
/// over REST. Only events of your tenant are sent; with a permission narrower than the whole
/// tenant you receive only the events you caused yourself. Only events a module declares
/// streamable are sent — never raw table changes.
///
/// Each event is one frame: `id: <event id>`, `event: <event name>`, `data: {"id", "name",
/// "occurred_at", "subject", "actor_kind", "payload"}`. A `: ping` comment comes every 15 s.
/// Delivery is **at least once**: after a reconnect you may receive an event again — skip ids you
/// already have. Reconnect with the `Last-Event-ID` header (browsers' `EventSource` does it for
/// you) or `last_event_id` to receive what you missed; events are kept 7 days, and when the id is
/// older the stream sends `event: reset` first — reload your data over REST. The server closes a
/// stream after 30 minutes and when it shuts down; reconnect the same way. Disabled unless the
/// server sets `AUTOXIDE_EVENTS_STREAM=true`.
#[utoipa::path(
    get,
    path = "/events/stream",
    operation_id = "stream_events",
    // Permission tuỳ event được nêu (`stream_permission` của từng event): ai đăng nhập cũng gọi được.
    extensions(("x-permissions" = json!([]))),
    security(("bearer" = [])),
    params(
        StreamQuery,
        ("Last-Event-ID" = Option<String>, Header, description = "The `id` of the last event you received; the stream resumes after it."),
    ),
    responses(
        (status = 200, description = "The stream, as `text/event-stream`; it stays open.", content_type = "text/event-stream", body = String),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = crate::response::Problem),
        (status = 403, description = "You may not receive an event you named (code `permission_denied`, naming the permission), or any of them.", body = crate::response::Problem),
        (status = 404, description = "Streaming is disabled on this server; code `kernel.stream_disabled`.", body = crate::response::Problem),
        (status = 422, description = "An event does not exist or is not streamable (code `kernel.stream_unknown_event`), or `names` / `last_event_id` is invalid (see `errors`).", body = crate::response::Problem),
        (status = 429, description = "Too many streams are open (per user, per tenant or on this server); code `kernel.stream_too_many`.", body = crate::response::Problem),
        (status = 503, description = "The server is shutting down; connect again; code `shutting_down`.", body = crate::response::Problem),
    )
)]
async fn stream_events(
    State(hub): State<StreamHub>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    if !hub.enabled() {
        return Err(disabled());
    }
    let filter = hub.filter(&tenant, &principal, &query.names)?;
    let header = headers.get("last-event-id").and_then(|value| value.to_str().ok());
    let last = header
        .or(query.last_event_id.as_deref())
        .filter(|raw| !raw.trim().is_empty())
        .map(parse_last_event_id)
        .transpose()?;
    let receiver = hub.open(&tenant, filter, last)?;
    // Khung đầu tiên đặt `retry:`; mỗi event một khung `id` / `event` / `data`.
    let first =
        futures_util::stream::once(async { Ok(Event::default().retry(RETRY).comment("open")) });
    let frames = futures_util::stream::unfold(receiver, |mut receiver| async move {
        let frame = receiver.recv().await?;
        Some((Ok::<Event, Infallible>(sse_event(&frame)), receiver))
    });
    let sse =
        Sse::new(first.chain(frames)).keep_alive(KeepAlive::new().interval(PING).text("ping"));
    let mut response = sse.into_response();
    // Nginx và một số proxy đệm thân response: tắt để event tới ngay.
    response.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

fn sse_event(frame: &Frame) -> Event {
    match frame {
        Frame::Event(event) => {
            let data = serde_json::to_string(event.as_ref()).unwrap_or_else(|_| "{}".to_owned());
            Event::default().id(event.id.to_string()).event(event.name.clone()).data(data)
        }
        Frame::Reset => Event::default().event("reset").data("{}"),
    }
}
