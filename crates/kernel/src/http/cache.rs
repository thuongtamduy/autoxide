//! `Cache-Control`, `ETag`, `Vary` cho cache dùng chung (CDN, reverse proxy) — bước 39 B5,
//! D-20261119-step39-cdn-cache.
//!
//! Handler vẫn quyết `Cache-Control` của mình; lớp này chỉ chốt ba luật để không response nào bị
//! cache sai chỗ:
//!
//! 1. Lệnh không phải `GET` / `HEAD` mà handler chưa đặt gì: `no-store`.
//! 2. Request mang `Authorization` hay cookie (người đã đăng nhập, hay trình duyệt có phiên): response
//!    **không bao giờ** `public` — chưa đặt gì thì `private, no-store`; handler đặt `public` thì đổi
//!    thành `private` và bỏ `s-maxage` / `stale-while-revalidate` (chỉ dành cho cache dùng chung).
//! 3. Đọc công khai không đăng nhập mà handler đánh dấu `public, no-cache` (quy ước của endpoint
//!    công khai: nội dung đổi được bất cứ lúc nào): thành
//!    `public, max-age=0, s-maxage=<AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS>, stale-while-revalidate=300`
//!    — trình duyệt luôn hỏi lại (rẻ nhờ `ETag` / `304`), CDN giữ tối đa chừng đó giây; thêm
//!    `Vary: Accept-Language, Accept-Encoding` (thông báo lỗi theo ngôn ngữ, bản nén); thiếu `ETag` thì thêm ETag yếu là
//!    dấu vân tay của thân (JSON tới 4 MiB), để lớp `304` ngoài cùng trả được `304`.
//!
//! Những gì handler đặt chặt hơn giữ nguyên: `no-store` của bản xem trước, file media có `?v=` đúng
//! phiên bản (`public, max-age=31536000, immutable`), `private` của file riêng tư.

use std::sync::Arc;

use axum::body::Body;
use axum::body::HttpBody as _;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};

/// Thân lớn hơn chừng này không đọc vào bộ nhớ để làm ETag.
const ETAG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// `stale-while-revalidate` của endpoint công khai: CDN trả bản cũ trong lúc hỏi lại máy chủ.
const STALE_SECONDS: u32 = 300;

/// Cấu hình đã dựng sẵn.
#[derive(Debug, Clone)]
pub(crate) struct Policy {
    /// `Cache-Control` của đọc công khai; `None` khi `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS=0` (giữ
    /// `public, no-cache` của handler).
    public: Option<HeaderValue>,
}

impl Policy {
    pub(crate) fn new(public_seconds: u32) -> Self {
        let public = (public_seconds > 0).then(|| {
            HeaderValue::from_str(&format!(
                "public, max-age=0, s-maxage={public_seconds}, stale-while-revalidate={STALE_SECONDS}"
            ))
            .unwrap_or_else(|_| HeaderValue::from_static("public, no-cache"))
        });
        Policy { public }
    }
}

pub(super) async fn cache_headers(policy: Arc<Policy>, request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let personal = request.headers().contains_key(header::AUTHORIZATION)
        || request.headers().contains_key(header::COOKIE);
    let response = next.run(request).await;
    finish(&policy, &method, personal, response).await
}

async fn finish(
    policy: &Policy,
    method: &Method,
    personal: bool,
    mut response: Response,
) -> Response {
    let reading = matches!(*method, Method::GET | Method::HEAD);
    let current = cache_control(response.headers());
    if !reading {
        if current.is_none() {
            set(&mut response, "no-store");
        }
        return response;
    }
    if personal {
        match current {
            None => set(&mut response, "private, no-store"),
            Some(value) if has(&value, "public") => {
                if let Ok(value) = HeaderValue::from_str(&privatize(&value)) {
                    response.headers_mut().insert(header::CACHE_CONTROL, value);
                }
            }
            Some(_) => {}
        }
        return response;
    }
    let marked_public = current.as_deref().is_some_and(|value| {
        let directives = directives(value);
        directives.len() == 2 && has(value, "public") && has(value, "no-cache")
    });
    if !marked_public {
        return response;
    }
    // `accept-encoding` cả khi thân nhỏ không nén: CDN cần biết trước bản lưu tách theo cách nén.
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("accept-language, accept-encoding"));
    if let Some(value) = &policy.public {
        response.headers_mut().insert(header::CACHE_CONTROL, value.clone());
    }
    if response.status() == StatusCode::OK && !response.headers().contains_key(header::ETAG) {
        response = with_fingerprint(response).await;
    }
    response
}

/// Thêm ETag yếu là dấu vân tay của thân (khi thân biết trước cỡ và không quá lớn).
async fn with_fingerprint(response: Response) -> Response {
    let small = response.body().size_hint().exact().is_some_and(|size| size <= ETAG_MAX_BYTES);
    if !small {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    // Cỡ đã biết và nhỏ: đọc trọn không lỗi trừ khi thân hỏng giữa chừng; khi đó trả thân rỗng
    // sẽ sai, nên trả 500 như mọi lỗi đọc thân khác.
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        let mut failed = Response::new(Body::empty());
        *failed.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        return failed;
    };
    let digest: String =
        Sha256::digest(&bytes).iter().take(16).map(|byte| format!("{byte:02x}")).collect();
    if let Ok(value) = HeaderValue::from_str(&format!("W/\"{digest}\"")) {
        parts.headers.insert(header::ETAG, value);
    }
    Response::from_parts(parts, Body::from(bytes))
}

fn set(response: &mut Response, value: &'static str) {
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
}

fn cache_control(headers: &HeaderMap) -> Option<String> {
    let values: Vec<&str> = headers
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect();
    (!values.is_empty()).then(|| values.join(", "))
}

fn directives(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|directive| directive.trim().to_ascii_lowercase())
        .filter(|directive| !directive.is_empty())
        .collect()
}

fn has(value: &str, name: &str) -> bool {
    directives(value).iter().any(|directive| directive.split('=').next() == Some(name))
}

/// `public` → `private`; bỏ chỉ thị chỉ dành cho cache dùng chung.
fn privatize(value: &str) -> String {
    directives(value)
        .into_iter()
        .filter_map(|directive| {
            let name = directive.split('=').next().unwrap_or_default().to_owned();
            match name.as_str() {
                "public" => Some("private".to_owned()),
                "s-maxage" | "stale-while-revalidate" | "proxy-revalidate" => None,
                _ => Some(directive),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_public_thanh_private() {
        assert_eq!(
            privatize("public, max-age=0, s-maxage=60, stale-while-revalidate=300"),
            "private, max-age=0"
        );
        assert_eq!(
            privatize("public, max-age=31536000, immutable"),
            "private, max-age=31536000, immutable"
        );
        assert!(has("Public, No-Cache", "no-cache"));
        assert!(!has("no-cache", "public"));
    }
}
