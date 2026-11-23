//! Request body size limit: the default `AUTOXIDE_HTTP_BODY_LIMIT_BYTES` for every
//! route; a route that needs a larger body (e.g. file upload) is declared separately by the module through
//! `Module::body_limits` (D-066).
//!
//! This layer sits inside `Router::layer`, i.e. it runs AFTER axum picks the route: it learns which route
//! through `MatchedPath`, like the deprecated-marking layer.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{MatchedPath, Request};
use axum::http::{Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use http_body_util::Limited;

use super::API_PREFIX;
use crate::error::{Error, ErrorKind};

/// A request body limit specific to one module route.
///
/// ```
/// use autoxide_kernel::http::BodyLimit;
/// use axum::http::Method;
///
/// // `POST /api/v1/files` accepts up to 25 MiB.
/// let limit = BodyLimit::new(Method::POST, "/", 25 * 1024 * 1024);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BodyLimit {
    pub method: Method,
    /// Path within the module, as when declaring the route (`/`, `/{id}`).
    pub path: &'static str,
    /// Maximum number of bytes of the request body.
    pub max_bytes: usize,
}

impl BodyLimit {
    pub fn new(method: Method, path: &'static str, max_bytes: usize) -> Self {
        BodyLimit { method, path, max_bytes }
    }
}

/// Lookup table (method, full path pattern) -> limit; a route without a declaration uses the default.
#[derive(Debug)]
pub(super) struct BodyLimits {
    default: usize,
    routes: HashMap<(Method, String), usize>,
}

impl BodyLimits {
    pub(super) fn new(default: usize, declared: &[(&'static str, BodyLimit)]) -> Self {
        let routes = declared
            .iter()
            .map(|(module, limit)| {
                let suffix = if limit.path == "/" { "" } else { limit.path };
                ((limit.method.clone(), format!("{API_PREFIX}/{module}{suffix}")), limit.max_bytes)
            })
            .collect();
        BodyLimits { default, routes }
    }

    /// The default limit (`AUTOXIDE_HTTP_BODY_LIMIT_BYTES`).
    pub(super) fn default_limit(&self) -> usize {
        self.default
    }

    /// The limit for `request`.
    pub(super) fn for_request(&self, request: &Request) -> usize {
        request
            .extensions()
            .get::<MatchedPath>()
            .and_then(|matched| {
                self.routes.get(&(request.method().clone(), matched.as_str().to_owned()))
            })
            .copied()
            .unwrap_or(self.default)
    }
}

/// The 413 error when the request body exceeds the limit.
pub(crate) fn too_large(limit: usize) -> Error {
    Error::new(
        ErrorKind::PayloadTooLarge,
        "payload_too_large",
        "The request body is larger than this endpoint accepts ({limit} bytes).",
    )
    .with_param("limit", limit)
}

/// HTTP layer: a `Content-Length` over the limit gets 413 immediately; without one, bytes are counted
/// while reading and it stops when the limit is exceeded.
pub(super) async fn limit(limits: Arc<BodyLimits>, request: Request, next: Next) -> Response {
    let limit = limits.for_request(&request);
    let declared = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if declared.is_some_and(|length| length > u64::try_from(limit).unwrap_or(u64::MAX)) {
        return too_large(limit).into_response();
    }
    next.run(request.map(|body| Body::new(Limited::new(body, limit)))).await
}

/// Whether `error` (an error while reading the request body) was caused by exceeding the limit.
pub fn is_too_large(error: &axum::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if current.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        source = current.source();
    }
    false
}
