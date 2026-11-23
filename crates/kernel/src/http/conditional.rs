//! `If-None-Match` becomes `304 Not Modified` for every `GET` / `HEAD` with an `ETag` (RFC 9110 §13.1.2).
//!
//! A client that already has the version carrying this `ETag` need not download the body again: this layer
//! turns a `200` response into a bodyless `304`, keeping the headers RFC 9110 requires (`ETag`,
//! `Cache-Control`, `Expires`, `Vary`, `Content-Location`, `Date`). Handlers need not do anything; a
//! handler that reads expensive data (e.g. file content) can check first with
//! [`crate::version::if_none_match`] to avoid the read.

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
