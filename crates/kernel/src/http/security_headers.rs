//! Security headers for every response (step 40 P1, D-320).
//!
//! This layer sits outside the log layer, inside the request id layer: every response passes through -
//! including the router's 404, rate limiting's 429, CORS preflight, `/health/*`, `/metrics`, `/mcp`. A header
//! is set only when the handler has **not** set it: a handler keeps the right to set a stricter value (e.g.
//! file download).

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::SecurityHeaders;

/// The API's default CSP: responses are JSON or user-uploaded files, nothing needs to be loaded; `sandbox`
/// stops uploaded HTML files from running scripts, even when opened directly in a browser.
pub const API_CSP: &str =
    "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; sandbox";

/// CSP of the API documentation page (`/api/docs`): the Scalar page loads scripts and styles from jsdelivr,
/// fonts from jsdelivr and `fonts.scalar.com` (checked in Chromium: without this source the fonts are
/// blocked, D-320), and calls `openapi.json` on the same origin. The `<script type="application/json">` block
/// is only data, and CSP does not block it.
pub const DOCS_CSP: &str = concat!(
    "default-src 'none'; ",
    "script-src https://cdn.jsdelivr.net; ",
    "style-src 'unsafe-inline' https://cdn.jsdelivr.net; ",
    "font-src https://cdn.jsdelivr.net https://fonts.scalar.com data:; ",
    "img-src 'self' data: https:; ",
    "connect-src 'self'; ",
    "frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
);

/// Headers prebuilt from configuration (built once, each response only copies them).
pub(crate) struct Prepared {
    common: Vec<(HeaderName, HeaderValue)>,
    api_csp: HeaderValue,
    docs_csp: HeaderValue,
}

impl Prepared {
    /// `None` when configuration disables security headers.
    pub(crate) fn new(config: &SecurityHeaders) -> Option<Arc<Self>> {
        if !config.enabled {
            return None;
        }
        let mut common = vec![
            (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
            (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
        ];
        // Cấu hình đã kiểm giá trị thuộc tập chuẩn (chữ ASCII); `if let` chỉ để khỏi `unwrap`.
        if let Ok(value) = HeaderValue::from_str(&config.referrer_policy) {
            common.push((header::REFERRER_POLICY, value));
        }
        if config.hsts_seconds > 0 {
            let mut value = format!("max-age={}", config.hsts_seconds);
            if config.hsts_include_subdomains {
                value.push_str("; includeSubDomains");
            }
            if let Ok(value) = HeaderValue::from_str(&value) {
                common.push((header::STRICT_TRANSPORT_SECURITY, value));
            }
        }
        Some(Arc::new(Prepared {
            common,
            api_csp: HeaderValue::from_static(API_CSP),
            docs_csp: HeaderValue::from_static(DOCS_CSP),
        }))
    }

    fn apply(&self, docs: bool, headers: &mut HeaderMap) {
        for (name, value) in &self.common {
            if !headers.contains_key(name) {
                headers.insert(name.clone(), value.clone());
            }
        }
        if !headers.contains_key(header::CONTENT_SECURITY_POLICY) {
            let csp = if docs { &self.docs_csp } else { &self.api_csp };
            headers.insert(header::CONTENT_SECURITY_POLICY, csp.clone());
        }
    }
}

/// The header-attaching layer. `prepared = None`: disabled by configuration, passes through unchanged.
pub(crate) async fn add(prepared: Option<Arc<Prepared>>, request: Request, next: Next) -> Response {
    let Some(prepared) = prepared else {
        return next.run(request).await;
    };
    // Đường dẫn phải lấy TRƯỚC khi chuyển request đi (`next.run` lấy quyền sở hữu request).
    let docs = request.uri().path() == super::DOCS_PATH;
    let mut response = next.run(request).await;
    prepared.apply(docs, response.headers_mut());
    response
}
