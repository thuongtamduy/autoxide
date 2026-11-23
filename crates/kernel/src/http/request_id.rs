//! Request ID: the `x-request-id` header.
//!
//! Used to trace a request through the logs and for users to report errors ("the request id is ...").

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

/// Name of the header carrying the request ID, on both request and response.
pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// A client-sent ID longer than this is dropped.
const MAX_CLIENT_ID_LENGTH: usize = 128;

/// Generates an ID for a request that has none: UUIDv7, so IDs sort by time.
#[derive(Debug, Clone, Copy)]
pub(super) struct MakeUuidV7;

impl MakeRequestId for MakeUuidV7 {
    fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
        HeaderValue::from_str(&Uuid::now_v7().to_string()).ok().map(RequestId::new)
    }
}

/// Drops a client-sent `x-request-id` if it is untrustworthy: too long, or containing characters other than
/// letters, digits and `-_.:`. This ID goes into the log; clients must not be able to inject arbitrary
/// content. The next layer generates a new ID for that request.
pub(super) async fn drop_untrusted(mut request: Request, next: Next) -> Response {
    let trusted = request.headers().get(&REQUEST_ID_HEADER).is_none_or(|value| {
        let bytes = value.as_bytes();
        !bytes.is_empty()
            && bytes.len() <= MAX_CLIENT_ID_LENGTH
            && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(byte))
    });
    if !trusted {
        request.headers_mut().remove(&REQUEST_ID_HEADER);
    }
    next.run(request).await
}
