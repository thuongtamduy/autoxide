//! Giới hạn kích thước thân request: mặc định `AUTOXIDE_HTTP_BODY_LIMIT_BYTES` cho mọi
//! route; route cần thân lớn hơn (vd tải file lên) được module khai riêng qua
//! `Module::body_limits` (D-066).
//!
//! Lớp này nằm trong `Router::layer`, tức chạy SAU khi axum chọn route: biết route nào
//! qua `MatchedPath`, như lớp đánh dấu deprecated.

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

/// Giới hạn thân request riêng cho một route của module.
///
/// ```
/// use autoxide_kernel::http::BodyLimit;
/// use axum::http::Method;
///
/// // `POST /api/v1/files` nhận tới 25 MiB.
/// let limit = BodyLimit::new(Method::POST, "/", 25 * 1024 * 1024);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyLimit {
    pub method: Method,
    /// Đường dẫn trong module, như khi khai route (`/`, `/{id}`).
    pub path: &'static str,
    /// Số byte tối đa của thân request.
    pub max_bytes: usize,
}

impl BodyLimit {
    pub fn new(method: Method, path: &'static str, max_bytes: usize) -> Self {
        BodyLimit { method, path, max_bytes }
    }
}

/// Bảng tra (method, mẫu đường dẫn đầy đủ) → giới hạn; route không khai dùng mặc định.
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

    /// Giới hạn cho `request`.
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

/// Lỗi 413 khi thân request vượt giới hạn.
pub(crate) fn too_large(limit: usize) -> Error {
    Error::new(
        ErrorKind::PayloadTooLarge,
        "payload_too_large",
        "The request body is larger than this endpoint accepts ({limit} bytes).",
    )
    .with_param("limit", limit)
}

/// Lớp HTTP: `Content-Length` quá giới hạn thì trả 413 ngay; không có thì đếm byte
/// trong lúc đọc và dừng khi vượt.
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

/// `error` (lỗi khi đọc thân request) có phải do vượt giới hạn không.
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
