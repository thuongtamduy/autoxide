//! Guarantees EVERY error response is Problem Details (RFC 9457).
//!
//! Errors created by Autoxide's own code (`Error`) already have the right shape. But libraries
//! also return errors on their own: no route (404), wrong method (405), body too large (413), a query
//! of the wrong type (400)... with an empty body or plain text. This layer converts them to the same
//! shape, so that clients only need to know one kind of error.

use std::any::Any;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tower_http::catch_panic::ResponseForPanic;

use crate::error::{Error, ErrorKind, FieldError};
use crate::response::{PROBLEM_CONTENT_TYPE, Problem};

/// A text error body longer than this is not copied into `detail`.
const MAX_DETAIL_BYTES: usize = 4096;

/// Marker for an error response whose body follows another standard on purpose (e.g. SCIM
/// errors, RFC 7644 §3.12): put it in the response extensions and the error is passed through
/// unchanged instead of being turned into Problem Details.
//
// Endpoint theo chuẩn ngành khác (SCIM) tự định dạng lỗi; lớp này chỉ chuẩn hoá lỗi do thư viện
// trả. Cộng thêm, không phá vỡ: response không mang marker vẫn như cũ (D-20261121-scim).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StandardErrorBody;

/// Converts an error response that is not yet Problem Details into Problem Details.
pub(super) async fn normalize(response: Response) -> Response {
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error())
        || is_problem(&response)
        || response.extensions().get::<StandardErrorBody>().is_some()
    {
        return response;
    }

    let is_text = content_type(&response).is_some_and(|value| value.starts_with("text/plain"));
    let (mut parts, body) = response.into_parts();
    // Thân chữ của lỗi 4xx do thư viện viết (vd "missing field `name`") là thông
    // tin giúp client sửa request: giữ lại làm `detail`. Lỗi 5xx thì không bao giờ
    // chép thân ra ngoài — nó có thể chứa chi tiết nội bộ.
    let detail = match axum::body::to_bytes(body, MAX_DETAIL_BYTES).await {
        Ok(bytes) if is_text && status.is_client_error() && !bytes.is_empty() => {
            String::from_utf8_lossy(&bytes).into_owned()
        }
        _ => default_detail(status).to_owned(),
    };
    // Query sai (thiếu tham số bắt buộc, sai kiểu) do `Query` của axum từ chối: trả như mọi lỗi
    // kiểm dữ liệu — `422 validation_failed` kèm ô sai trong `errors`, dịch theo ngôn ngữ.
    if status == StatusCode::BAD_REQUEST
        && let Some(rest) = detail.strip_prefix(QUERY_REJECTION)
    {
        return Error::validation(vec![query_problem(rest)]).into_response();
    }

    let problem = Problem {
        type_uri: "about:blank".to_owned(),
        title: status.canonical_reason().unwrap_or("Error").to_owned(),
        status: status.as_u16(),
        code: code_for(status).to_owned(),
        detail: detail.clone(),
        errors: Vec::new(),
        reason: None,
        current: None,
    };
    // Gắn lỗi tương ứng vào extensions, như mọi lỗi khác: lớp chọn ngôn ngữ dịch theo
    // `code` của nó. (Loại lỗi ở đây không quyết định gì: trạng thái giữ như response.)
    let kind = if status.is_server_error() { ErrorKind::Internal } else { ErrorKind::BadRequest };
    parts.extensions.insert(Arc::new(Error::new(kind, code_for(status), detail)));
    // Giữ các header khác của response gốc (vd `allow` của lỗi 405).
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_CONTENT_TYPE));
    let body = serde_json::to_vec(&problem).unwrap_or_default();
    Response::from_parts(parts, Body::from(body))
}

/// The beginning of the sentence axum's `Query` writes when it cannot read the query string.
const QUERY_REJECTION: &str = "Failed to deserialize query string: ";

/// The offending field from a library sentence: "missing field `locale`" -> `locale` is required; "limit:
/// invalid digit..." -> `limit` is wrong; if no field is identifiable -> `query`.
fn query_problem(text: &str) -> FieldError {
    if let Some(name) = text
        .split("missing field `")
        .nth(1)
        .and_then(|rest| rest.split('`').next())
        .filter(|name| !name.is_empty())
    {
        return FieldError::new(name.to_owned(), "required", "is required");
    }
    match text.split_once(": ") {
        Some((path, _))
            if !path.is_empty()
                && path.chars().all(|c| c.is_ascii_alphanumeric() || "_.[]".contains(c)) =>
        {
            FieldError::new(path.to_owned(), "invalid", "has an invalid value")
        }
        _ => FieldError::new("query", "invalid", "has an invalid value"),
    }
}

fn content_type(response: &Response) -> Option<&str> {
    response.headers().get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok())
}

fn is_problem(response: &Response) -> bool {
    content_type(response).is_some_and(|value| value.starts_with(PROBLEM_CONTENT_TYPE))
}

/// Stable error code for an HTTP status returned by a library.
/// The code for an error status not in the table below.
const FALLBACK_CODE: &str = "http_error";

fn code_for(status: StatusCode) -> &'static str {
    match status.as_u16() {
        400 => "bad_request",
        401 => "unauthenticated",
        403 => "forbidden",
        404 => "not_found",
        405 => "method_not_allowed",
        406 => "not_acceptable",
        408 => "request_timeout",
        409 => "conflict",
        411 => "length_required",
        413 => "payload_too_large",
        414 => "uri_too_long",
        415 => "unsupported_media_type",
        422 => "invalid_request",
        429 => "too_many_requests",
        431 => "headers_too_large",
        500 => "internal_error",
        501 => "not_implemented",
        503 => "unavailable",
        504 => "gateway_timeout",
        _ => FALLBACK_CODE,
    }
}

fn default_detail(status: StatusCode) -> &'static str {
    match status.as_u16() {
        404 => "No such endpoint. Check the path against the OpenAPI document.",
        405 => "This endpoint does not support this HTTP method. See the `allow` header.",
        413 => "The request body is larger than the server accepts.",
        415 => "Unsupported content type. Send JSON with `content-type: application/json`.",
        status if status >= 500 => {
            "An internal error occurred. Retry later; if it persists, report the request id."
        }
        _ => "The request could not be processed.",
    }
}

/// Converts a panic in a handler into a properly shaped 500 error. The panic content goes into the error
/// (for the log), and does not reach the client.
#[derive(Debug, Clone, Copy)]
pub(super) struct PanicToProblem;

impl ResponseForPanic for PanicToProblem {
    type ResponseBody = Body;

    fn response_for_panic(&mut self, panic: Box<dyn Any + Send + 'static>) -> Response {
        // Giá trị của một panic thường là `&str` hoặc `String`.
        let message = panic
            .downcast_ref::<&str>()
            .map(|text| (*text).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_owned());
        let mut response = Error::internal(format!("handler panicked: {message}")).into_response();
        // Handler chết giữa chừng: việc có thể đã xong (D-157).
        response.extensions_mut().insert(super::idempotency::OutcomeUnknown);
        response
    }
}

#[cfg(test)]
mod standard_error_body_tests {
    use super::*;

    #[tokio::test]
    async fn loi_mang_marker_giu_nguyen_than() {
        // Lỗi theo chuẩn khác (SCIM): giữ nguyên thân và content-type.
        let mut response = (StatusCode::BAD_REQUEST, "{\"status\":\"400\"}").into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static("application/scim+json"));
        response.extensions_mut().insert(StandardErrorBody);
        let kept = normalize(response).await;
        assert_eq!(kept.headers()[header::CONTENT_TYPE], "application/scim+json");
        let body = axum::body::to_bytes(kept.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"{\"status\":\"400\"}");

        // Không marker: vẫn đổi thành Problem như trước.
        let plain = (StatusCode::BAD_REQUEST, "oops").into_response();
        let changed = normalize(plain).await;
        assert!(is_problem(&changed));
    }
}
