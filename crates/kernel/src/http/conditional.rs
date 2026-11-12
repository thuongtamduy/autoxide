//! `If-None-Match` → `304 Not Modified` cho mọi `GET` / `HEAD` có `ETag` (RFC 9110 §13.1.2).
//!
//! Client đã có bản mang `ETag` này thì không cần tải lại thân: lớp này đổi response
//! `200` thành `304` không thân, giữ những header RFC 9110 bắt giữ (`ETag`,
//! `Cache-Control`, `Expires`, `Vary`, `Content-Location`, `Date`). Handler không phải
//! làm gì; handler đọc dữ liệu đắt (vd nội dung file) có thể tự kiểm trước bằng
//! [`crate::version::if_none_match`] để khỏi đọc.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::version::if_none_match;

pub(super) async fn not_modified(request: Request, next: Next) -> Response {
    let conditional = matches!(*request.method(), Method::GET | Method::HEAD)
        && request.headers().contains_key(header::IF_NONE_MATCH);
    if !conditional {
        return next.run(request).await;
    }
    let headers = request.headers().clone();
    let response = next.run(request).await;
    if response.status() != StatusCode::OK {
        return response;
    }
    let Some(etag) = response.headers().get(header::ETAG).and_then(|v| v.to_str().ok()) else {
        return response;
    };
    if !if_none_match(&headers, etag) {
        return response;
    }
    let (parts, _) = response.into_parts();
    let mut not_modified = Response::new(Body::empty());
    *not_modified.status_mut() = StatusCode::NOT_MODIFIED;
    for name in [
        header::ETAG,
        header::CACHE_CONTROL,
        header::EXPIRES,
        header::VARY,
        header::CONTENT_LOCATION,
        header::DATE,
    ] {
        if let Some(value) = parts.headers.get(&name) {
            not_modified.headers_mut().insert(name, value.clone());
        }
    }
    not_modified
}
