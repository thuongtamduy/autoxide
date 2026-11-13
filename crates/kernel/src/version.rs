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
//! - Không gửi `If-Match` thì sửa như cũ (ghi đè), để client cũ vẫn chạy — trừ endpoint
//!   gọi [`IfMatch::required`]: không gửi là 428 `precondition_required` (RFC 6585).
//! - Đọc: client gửi lại `ETag` trong `If-None-Match`; còn khớp thì lớp HTTP của kernel
//!   trả `304 Not Modified` không kèm thân (xem [`if_none_match`]) — mọi `GET` có `ETag`
//!   đều được, không phải viết gì thêm.

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
#[non_exhaustive]
pub enum IfMatch {
    /// Không gửi header.
    Missing,
    /// Gửi `*`: sửa không cần kiểm phiên bản.
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
            IfMatch::Missing | IfMatch::Any => Ok(None),
            IfMatch::Version(version) => Ok(Some(version)),
            IfMatch::Never => Err(changed()),
        }
    }

    /// Như [`expected`](Self::expected), nhưng bắt buộc gửi header: thiếu thì 428.
    /// `*` vẫn được nhận — người gọi nói rõ là muốn ghi đè.
    pub fn required(self) -> Result<Option<DateTime<Utc>>, Error> {
        match self {
            IfMatch::Missing => Err(Error::new(
                ErrorKind::PreconditionRequired,
                "precondition_required",
                "Send the `ETag` you read in an `If-Match` header, so that the change is not \
                 made over someone else's (`If-Match: *` to change it whatever its version).",
            )),
            other => other.expected(),
        }
    }

    /// Đọc một giá trị `ETag` (hoặc `*`) nhận qua kênh khác header — vd tham số `version` của
    /// tool MCP — y như header `If-Match`.
    pub fn from_value(value: &str) -> IfMatch {
        IfMatch::parse(Some(value))
    }

    fn parse(value: Option<&str>) -> IfMatch {
        let Some(value) = value.map(str::trim) else {
            return IfMatch::Missing;
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
            None => Ok(IfMatch::Missing),
            Some(value) => Ok(IfMatch::parse(Some(value.to_str().unwrap_or_default()))),
        }
    }
}

/// Header `If-None-Match` của request có khớp `etag` (một giá trị `ETag` đầy đủ, kể cả
/// ngoặc kép) không: so sánh "yếu" của RFC 9110 (bỏ qua tiền tố `W/`), nhận danh sách
/// nhiều giá trị và `*`. Khớp nghĩa là client đã có đúng bản này: trả 304.
pub fn if_none_match(headers: &axum::http::HeaderMap, etag: &str) -> bool {
    let wanted = etag.trim_start_matches("W/");
    headers.get_all(header::IF_NONE_MATCH).iter().filter_map(|value| value.to_str().ok()).any(
        |value| {
            value
                .split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate.trim_start_matches("W/") == wanted)
        },
    )
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
        assert_eq!(IfMatch::parse(None), IfMatch::Missing);
        assert_eq!(IfMatch::Missing.expected().unwrap(), None);
        let required = IfMatch::Missing.required().unwrap_err();
        assert_eq!(
            (required.kind(), required.code()),
            (ErrorKind::PreconditionRequired, "precondition_required")
        );
        assert_eq!(IfMatch::Any.required().unwrap(), None);
        assert_eq!(IfMatch::parse(Some("*")), IfMatch::Any);
        for wrong in ["W/\"1\"", "\"abc\"", "1", "\"1\", \"2\"", ""] {
            assert_eq!(IfMatch::parse(Some(wrong)), IfMatch::Never, "{wrong}");
        }
        assert_eq!(IfMatch::Never.expected().unwrap_err().kind(), ErrorKind::PreconditionFailed);
    }

    #[test]
    fn if_none_match_so_yeu_va_danh_sach() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!if_none_match(&headers, "\"a\""));
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"x\", W/\"a\""));
        assert!(if_none_match(&headers, "\"a\""));
        assert!(!if_none_match(&headers, "\"b\""));
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
        assert!(if_none_match(&headers, "\"b\""));
    }

    #[tokio::test]
    async fn ket_qua_kem_etag() {
        let version = DateTime::from_timestamp_micros(1_700_000_000_123_456).unwrap();
        let response = Tagged { version, data: 1 }.into_response();
        assert_eq!(response.headers()[header::ETAG], "\"1700000000123456\"");
    }
}
