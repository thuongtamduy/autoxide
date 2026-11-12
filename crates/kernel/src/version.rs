//! Chống ghi đè khi hai người cùng sửa một bản ghi (optimistic concurrency), theo HTTP
//! chuẩn (RFC 9110): `ETag` khi đọc, `If-Match` khi sửa.
//!
//! Phiên bản của một bản ghi là cột `updated_at` của nó (đã cắt tới micro giây nên hai
//! database lưu cùng một giá trị). `ETag` là số micro giây đó trong ngoặc kép.
//!
//! - Handler đọc / sửa trả [`Tagged`] thay cho `Data`: thân như `Data`, kèm `ETag`.
//! - Handler sửa nhận [`IfMatch`] và đưa [`IfMatch::expected`] cho service. Service sửa
//!   bằng một câu `UPDATE … WHERE updated_at = <phiên bản mong đợi>`: 0 dòng nghĩa là bản
//!   ghi vừa bị người khác sửa → [`changed`] (412). Kiểm trong chính câu `UPDATE` nên hai
//!   request đồng thời không đè nhau được, trên cả hai database.
//! - Không gửi `If-Match` thì sửa như cũ (ghi đè), để client cũ vẫn chạy.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::error::{Error, ErrorKind};
use crate::response::Data;

/// Giá trị `ETag` của phiên bản `version`: `"<số micro giây Unix>"`.
pub fn etag(version: DateTime<Utc>) -> String {
    format!("\"{}\"", version.timestamp_micros())
}

/// Kết quả `{"data": …}` kèm header `ETag` của bản ghi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged<T> {
    pub version: DateTime<Utc>,
    pub data: T,
}

impl<T: Serialize> IntoResponse for Tagged<T> {
    fn into_response(self) -> Response {
        let mut response = Data { data: self.data }.into_response();
        if let Ok(value) = HeaderValue::from_str(&etag(self.version)) {
            response.headers_mut().insert(header::ETAG, value);
        }
        response
    }
}

/// Header `If-Match` của request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IfMatch {
    /// Không gửi, hoặc gửi `*`: sửa không cần kiểm phiên bản.
    Any,
    /// Chỉ sửa khi bản ghi còn đúng phiên bản này.
    Version(DateTime<Utc>),
    /// Giá trị không phải `ETag` máy chủ này cấp (sai dạng, `ETag` yếu `W/…`, nhiều giá trị):
    /// không bao giờ khớp.
    Never,
}

impl IfMatch {
    /// Phiên bản service phải khớp (`None`: không kiểm); lỗi 412 nếu không thể khớp.
    pub fn expected(self) -> Result<Option<DateTime<Utc>>, Error> {
        match self {
            IfMatch::Any => Ok(None),
            IfMatch::Version(version) => Ok(Some(version)),
            IfMatch::Never => Err(changed()),
        }
    }

    fn parse(value: Option<&str>) -> IfMatch {
        let Some(value) = value.map(str::trim) else {
            return IfMatch::Any;
        };
        if value == "*" {
            return IfMatch::Any;
        }
        value
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .and_then(|micros| micros.parse::<i64>().ok())
            .and_then(DateTime::from_timestamp_micros)
            .map_or(IfMatch::Never, IfMatch::Version)
    }
}

// Extractor của axum: handler khai `if_match: IfMatch` là nhận được header đã đọc.
impl<S: Send + Sync> FromRequestParts<S> for IfMatch {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Error> {
        let mut values = parts.headers.get_all(header::IF_MATCH).iter();
        let first = values.next();
        if values.next().is_some() {
            return Ok(IfMatch::Never);
        }
        match first {
            None => Ok(IfMatch::Any),
            Some(value) => Ok(IfMatch::parse(Some(value.to_str().unwrap_or_default()))),
        }
    }
}

/// Lỗi 412: bản ghi đã bị sửa từ lúc người gọi đọc nó.
pub fn changed() -> Error {
    Error::new(
        ErrorKind::PreconditionFailed,
        "precondition_failed",
        "The record changed since you read it (its `ETag` is different). Get it again, reapply \
         your change, and send the new `ETag` in `If-Match`.",
    )
}

#[cfg(test)]
mod tests {
    use chrono::SubsecRound;

    use super::*;

    #[test]
    fn etag_doc_lai_dung_phien_ban() {
        let version = Utc::now().trunc_subsecs(6);
        assert_eq!(IfMatch::parse(Some(&etag(version))), IfMatch::Version(version));
        assert_eq!(IfMatch::parse(None), IfMatch::Any);
        assert_eq!(IfMatch::parse(Some("*")), IfMatch::Any);
        for wrong in ["W/\"1\"", "\"abc\"", "1", "\"1\", \"2\"", ""] {
            assert_eq!(IfMatch::parse(Some(wrong)), IfMatch::Never, "{wrong}");
        }
        assert_eq!(IfMatch::Never.expected().unwrap_err().kind(), ErrorKind::PreconditionFailed);
    }

    #[tokio::test]
    async fn ket_qua_kem_etag() {
        let version = DateTime::from_timestamp_micros(1_700_000_000_123_456).unwrap();
        let response = Tagged { version, data: 1 }.into_response();
        assert_eq!(response.headers()[header::ETAG], "\"1700000000123456\"");
    }
}
