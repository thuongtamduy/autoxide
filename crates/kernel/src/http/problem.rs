//! Bảo đảm MỌI response lỗi là Problem Details (RFC 9457).
//!
//! Lỗi do code của Autoxide tạo (`Error`) vốn đã đúng dạng. Nhưng thư viện cũng
//! tự trả lỗi: không có route (404), sai method (405), body quá lớn (413), query
//! sai kiểu (400)… với thân rỗng hoặc chữ thường. Lớp này đổi chúng về cùng một
//! dạng, để client chỉ phải biết đúng một kiểu lỗi.

use std::any::Any;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tower_http::catch_panic::ResponseForPanic;

use crate::error::{Error, ErrorKind, FieldError};
use crate::response::{PROBLEM_CONTENT_TYPE, Problem};

/// Thân lỗi dạng chữ dài hơn mức này thì không chép vào `detail`.
const MAX_DETAIL_BYTES: usize = 4096;

/// Đổi response lỗi chưa phải Problem Details thành Problem Details.
pub(super) async fn normalize(response: Response) -> Response {
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) || is_problem(&response) {
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

/// Đầu câu `Query` của axum viết khi không đọc được query string.
const QUERY_REJECTION: &str = "Failed to deserialize query string: ";

/// Ô sai từ câu của thư viện: "missing field `locale`" → `locale` bắt buộc; "limit: invalid digit…"
/// → `limit` sai; không rõ ô nào → `query`.
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

/// Mã lỗi ổn định cho một mã trạng thái HTTP do thư viện trả.
/// Mã cho trạng thái lỗi không có trong bảng dưới.
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

/// Đổi một panic trong handler thành lỗi 500 đúng dạng. Nội dung panic đi vào lỗi
/// (cho log), không ra tới client.
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
