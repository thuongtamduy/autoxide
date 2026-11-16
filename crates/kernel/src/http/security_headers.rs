//! Header bảo mật cho mọi response (bước 40 P1, D-320).
//!
//! Lớp này đặt ngoài lớp log, trong lớp request id: mọi response đều đi qua — kể cả 404 của router,
//! 429 của rate limiting, preflight CORS, `/health/*`, `/metrics`, `/mcp`. Header chỉ được đặt khi
//! handler **chưa** đặt: handler giữ quyền đặt giá trị chặt hơn (vd tải file).

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::config::SecurityHeaders;

/// CSP mặc định của API: response là JSON hay file người dùng tải lên, không cần nạp gì; `sandbox`
/// làm file HTML tải lên không chạy được script, kể cả khi mở thẳng trong trình duyệt.
pub const API_CSP: &str =
    "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; sandbox";

/// CSP của trang tài liệu API (`/api/docs`): trang Scalar nạp script, style từ jsdelivr, font từ
/// jsdelivr và `fonts.scalar.com` (đã kiểm trong Chromium: thiếu nguồn này thì font bị chặn, D-320),
/// và gọi `openapi.json` cùng origin. Khối `<script type="application/json">` chỉ là dữ liệu, CSP
/// không chặn.
pub const DOCS_CSP: &str = concat!(
    "default-src 'none'; ",
    "script-src https://cdn.jsdelivr.net; ",
    "style-src 'unsafe-inline' https://cdn.jsdelivr.net; ",
    "font-src https://cdn.jsdelivr.net https://fonts.scalar.com data:; ",
    "img-src 'self' data: https:; ",
    "connect-src 'self'; ",
    "frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
);

/// Header đã dựng sẵn từ cấu hình (dựng một lần, mỗi response chỉ chép).
pub(crate) struct Prepared {
    common: Vec<(HeaderName, HeaderValue)>,
    api_csp: HeaderValue,
    docs_csp: HeaderValue,
}

impl Prepared {
    /// `None` khi cấu hình tắt header bảo mật.
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

/// Lớp gắn header. `prepared = None`: cấu hình tắt, cho qua nguyên vẹn.
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
