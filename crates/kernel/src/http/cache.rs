//! `Cache-Control`, `ETag` and `Vary` for shared caches (CDN, reverse proxy) - step 39 B5,
//! D-20261119-step39-cdn-cache.
//!
//! Handlers still decide their own `Cache-Control`; this layer only enforces three rules so that no
//! response is cached in the wrong place:
//!
//! 1. Write requests (not `GET` / `HEAD`): nothing changes. Shared caches do not keep responses to write
//!    requests anyway (RFC 9111 §9); and `no-store` on a write request has its own meaning in the
//!    idempotency layer ("the response contains a secret, do not store it for replay"), so this layer must
//!    not set it itself.
//! 2. A request carrying `Authorization` or a cookie (a signed-in user, or a browser with a session): the
//!    response is **never** `public` - if nothing is set it becomes `private, no-store`; if the handler
//!    sets `public` it is changed to `private` and `s-maxage` / `stale-while-revalidate` are dropped (they
//!    only apply to shared caches).
//! 3. An anonymous public read that the handler marks `public, no-cache` (the convention for public
//!    endpoints: the content can change at any time): becomes
//!    `public, max-age=0, s-maxage=<AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS>, stale-while-revalidate=300` Browsers
//!    always revalidate (cheap thanks to `ETag` / `304`), the CDN keeps it for at most that many seconds; it
//!    also adds `Vary: Accept-Language, Accept-Encoding` (error messages depend on language, and the body may
//!    be compressed); if there is no `ETag`, it adds a weak ETag that is a fingerprint of the body (JSON up
//!    to 4 MiB), so that the outermost `304` layer can return a `304`.
//!
//! Anything the handler sets more strictly is kept: `no-store` for previews, media files with a `?v=`
//! matching the version (`public, max-age=31536000, immutable`), `private` for private files.

use std::sync::Arc;

use axum::body::Body;
use axum::body::HttpBody as _;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use sha2::{Digest, Sha256};

/// A body larger than this is not read into memory to compute an ETag.
const ETAG_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// `stale-while-revalidate` for public endpoints: the CDN serves the stale copy while revalidating with the
/// server.
const STALE_SECONDS: u32 = 300;

/// The pre-built configuration.
#[derive(Debug, Clone)]
pub(crate) struct Policy {
    /// The `Cache-Control` for public reads; `None` when `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS=0` (keeping
    /// the handler's `public, no-cache`).
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

/// Adds a weak ETag that is a fingerprint of the body (when the body's size is known up front and not too
/// large).
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

/// `public` becomes `private`; drops directives that only apply to shared caches.
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
