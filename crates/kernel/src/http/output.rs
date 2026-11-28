//! Responses besides the envelopes of [`response`](crate::response): [`NoContent`], [`Download`] and the
//! [`NoStore`] wrapper.

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

/// A response that carries a secret shown only once (a new API key, a webhook signing secret): the wrapped
/// response with `Cache-Control: no-store`, so no proxy or browser keeps it and the idempotency layer does not
/// store it in the database - a retry with the same `Idempotency-Key` gets `409 idempotency_result_not_kept`
/// instead of the secret again. `Ok(NoStore(Created::new(path, data)))`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoStore<T>(pub T);

impl<T: IntoResponse> IntoResponse for NoStore<T> {
    fn into_response(self) -> Response {
        let mut response = self.0.into_response();
        response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
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
        let response = NoStore(NoContent).into_response();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        // Content-Type sai cú pháp: không lỗi, dùng kiểu nhị phân chung.
        let response = Download::new(Vec::new(), "bad\nvalue", "x").into_response();
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/octet-stream");
    }
}
