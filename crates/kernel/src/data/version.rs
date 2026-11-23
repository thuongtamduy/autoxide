//! Preventing overwrites when two people edit the same record (optimistic concurrency), using standard HTTP
//! (RFC 9110): `ETag` when reading, `If-Match` when editing.
//!
//! The version of a record is its `updated_at` column (truncated to microseconds so both databases store
//! the same value). The `ETag` is that microsecond number in double quotes.
//!
//! - Read / edit handlers return [`Tagged`] instead of `Data`: the body is like `Data`, with an `ETag`.
//! - An edit handler takes [`IfMatch`] and passes [`IfMatch::expected`] to the service. The service edits with
//!   one statement `UPDATE … WHERE updated_at = <expected version>`: 0 rows means the record was just edited by
//!   someone else → [`changed`] (412). The check is inside the `UPDATE` itself, so two concurrent requests
//!   cannot overwrite each other, on both databases.
//! - Without `If-Match`, the edit works as before (overwrites), so old clients keep working, except endpoints
//!   that call [`IfMatch::required`]: omitting it is 428 `precondition_required` (RFC 6585).
//! - Reading: the client sends the `ETag` back in `If-None-Match`; if it still matches, the kernel's HTTP layer
//!   returns `304 Not Modified` with no body (see [`if_none_match`]); every `GET` with an `ETag` qualifies,
//!   with nothing more to write.

use crate::time::Timestamp;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::error::{Error, ErrorKind};
use crate::response::Data;

/// The `ETag` value of version `version`: `"<Unix microsecond number>"`.
pub fn etag(version: Timestamp) -> String {
    format!("\"{}\"", version.as_unix_micros())
}

/// A `{"data": …}` result with the record's `ETag` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tagged<T> {
    pub version: Timestamp,
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

/// The `If-Match` header of the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum IfMatch {
    /// The header was not sent.
    Missing,
    /// `*` was sent: edit without a version check.
    Any,
    /// Edit only if the record is still at exactly this version.
    Version(Timestamp),
    /// A value that is not an `ETag` this server issued (malformed, weak `ETag` `W/…`, several values):
    /// never matches.
    Never,
}

impl IfMatch {
    /// The version the service must match (`None`: no check); a 412 error if it cannot match.
    pub fn expected(self) -> Result<Option<Timestamp>, Error> {
        match self {
            IfMatch::Missing | IfMatch::Any => Ok(None),
            IfMatch::Version(version) => Ok(Some(version)),
            IfMatch::Never => Err(changed()),
        }
    }

    /// Like [`expected`](Self::expected), but the header is required: missing gives 428.
    /// `*` is still accepted; the caller states explicitly that they want to overwrite.
    pub fn required(self) -> Result<Option<Timestamp>, Error> {
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

    /// Reads an `ETag` value (or `*`) received through a channel other than a header (e.g. the `version`
    /// parameter of an MCP tool), exactly like the `If-Match` header.
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
            .and_then(Timestamp::from_unix_micros)
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

/// Whether the request's `If-None-Match` header matches `etag` (a full `ETag` value, double quotes included):
/// the "weak" comparison of RFC 9110 (ignores the `W/` prefix), accepting a list of several values and `*`.
/// A match means the client already has exactly this version: return 304.
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

/// 412 error: the record was edited since the caller read it.
pub fn changed() -> Error {
    Error::new(
        ErrorKind::PreconditionFailed,
        "precondition_failed",
        "The record changed since you read it (its `ETag` is different). Get it again, reapply \
         your change, and send the new `ETag` in `If-Match`.",
    )
}

/// A 412 error like [`changed`], with the current record `current` (in the form it is read) and its version
/// `version` (step 69 P2): an offline app merges its own changes into the server version itself and resends
/// with the new `ETag`, without re-reading. If `current` cannot be turned into JSON, it still returns 412,
/// without the record.
pub fn changed_with<T: Serialize>(current: &T, version: Timestamp) -> Error {
    match serde_json::to_value(current) {
        Ok(record) => changed().with_current(record, version),
        Err(_) => changed(),
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn etag_doc_lai_dung_phien_ban() {
        let version = Timestamp::now();
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
        let version = Timestamp::from_unix_micros(1_700_000_000_123_456).unwrap();
        let response = Tagged { version, data: 1 }.into_response();
        assert_eq!(response.headers()[header::ETAG], "\"1700000000123456\"");
    }
}
