//! Responses besides the envelopes of [`response`](crate::response): [`NoContent`], [`Download`] and
//! [`PublicCache`].

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::{Disposition, content_disposition};

/// `204 No Content`: the request succeeded and there is nothing to send back (a delete, for example).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoContent;

impl IntoResponse for NoContent {
    fn into_response(self) -> Response {
        StatusCode::NO_CONTENT.into_response()
    }
}

/// A file to download: its bytes, media type and file name (`Content-Disposition: attachment`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Download {
    /// The content of the file.
    pub bytes: Vec<u8>,
    /// Its media type, e.g. `text/csv; charset=utf-8`.
    pub content_type: String,
    /// The name the client saves it as.
    pub file_name: String,
}

impl Download {
    /// A file named `file_name` of type `content_type`.
    pub fn new(
        bytes: Vec<u8>,
        content_type: impl Into<String>,
        file_name: impl Into<String>,
    ) -> Self {
        Download { bytes, content_type: content_type.into(), file_name: file_name.into() }
    }
}

impl IntoResponse for Download {
    fn into_response(self) -> Response {
        let content_type = HeaderValue::from_str(&self.content_type)
            .unwrap_or(HeaderValue::from_static("application/octet-stream"));
        let disposition = content_disposition(Disposition::Attachment, &self.file_name);
        let mut response = (StatusCode::OK, self.bytes).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, content_type);
        if let Ok(value) = HeaderValue::from_str(&disposition) {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
        response
    }
}

/// A public, anonymous read that caches may keep for `max_age` seconds (`Cache-Control: public, max-age=…`), with
/// an optional `ETag` — e.g. a reference list that changes only when an administrator imports a new one. The
/// kernel's cache layer still turns `public` into `private` for a signed-in request, and answers `304` when
/// `If-None-Match` matches the `ETag`. Wraps any result a handler returns (`Data`, `Page`).
///
/// The handler picks `max_age` itself, so `AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS` does not apply and no
/// `Vary: Accept-Language` is added: use it only for bodies that do not depend on the language and that may be
/// kept that long. Content that can change at any time uses the `public, no-cache` convention of the cache layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicCache<T> {
    body: T,
    etag: Option<String>,
    max_age: u32,
}

impl<T> PublicCache<T> {
    /// `body`, cacheable for `max_age` seconds.
    pub fn new(body: T, max_age: u32) -> Self {
        PublicCache { body, etag: None, max_age }
    }

    /// With the `ETag` header `etag` (a quoted string such as `"abc123"`); `None` sends none. A value that is not
    /// a valid header is left out.
    #[must_use]
    pub fn with_etag(mut self, etag: Option<String>) -> Self {
        self.etag = etag;
        self
    }
}

// Cài trait của axum cho kiểu kernel: chi tiết bên trong, ngoài hợp đồng (D-20261128-library-layer-plan mục 1).
impl<T: IntoResponse> IntoResponse for PublicCache<T> {
    fn into_response(self) -> Response {
        let mut response = self.body.into_response();
        let headers = response.headers_mut();
        if let Some(value) = self.etag.and_then(|etag| HeaderValue::from_str(&etag).ok()) {
            headers.insert(header::ETAG, value);
        }
        if let Ok(value) = HeaderValue::from_str(&format!("public, max-age={}", self.max_age)) {
            headers.insert(header::CACHE_CONTROL, value);
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tra_204_va_tep_tai_ve() {
        assert_eq!(NoContent.into_response().status(), StatusCode::NO_CONTENT);
        let response = Download::new(b"a,b\n".to_vec(), "text/csv; charset=utf-8", "đơn hàng.csv")
            .into_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/csv; charset=utf-8");
        let disposition =
            response.headers()[header::CONTENT_DISPOSITION].to_str().unwrap_or_default();
        assert!(disposition.starts_with("attachment;"), "{disposition}");
        // Content-Type sai cú pháp: không lỗi, dùng kiểu nhị phân chung.
        let response = Download::new(Vec::new(), "bad\nvalue", "x").into_response();
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/octet-stream");
    }

    #[test]
    fn cache_cong_khai_them_etag_va_cache_control() {
        let response = PublicCache::new(NoContent, 86_400)
            .with_etag(Some("\"abc\"".to_owned()))
            .into_response();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::ETAG], "\"abc\"");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "public, max-age=86400");
        let response = PublicCache::new(NoContent, 60)
            .with_etag(Some("bad\nvalue".to_owned()))
            .into_response();
        assert!(response.headers().get(header::ETAG).is_none());
        assert_eq!(response.headers()[header::CACHE_CONTROL], "public, max-age=60");
    }
}
