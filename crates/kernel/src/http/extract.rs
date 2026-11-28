//! Extracts a handler's input.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use serde::de::DeserializeOwned;

use super::Validate;
use crate::error::{Error, ErrorKind};

/// The JSON body of the request, read as `T` AND having passed [`Validate::check`].
///
/// When a handler takes `ValidJson(input): ValidJson<CreateNote>`, `input` is guaranteed valid; a bad
/// request gets an error before the handler runs:
///
/// - not JSON, or malformed JSON: 400 `invalid_json`;
/// - missing `content-type: application/json`: 415 `unsupported_media_type`;
/// - missing field, wrong type: 422 `invalid_body`;
/// - a value that fails validation: 422 `validation_failed` with the error for each field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate,
{
    type Rejection = Error;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        // Đọc thân một lần để từ chối ký tự NUL trước khi đọc thành `T` (D-157); rồi để `Json`
        // của axum làm phần còn lại như cũ (kiểm `content-type`, lỗi cú pháp, lỗi dữ liệu).
        let (parts, body) = request.into_parts();
        let bytes =
            axum::body::Bytes::from_request(Request::from_parts(parts.clone(), body), state)
                .await
                .map_err(|rejection| {
                    if rejection.status().as_u16() == 413 {
                        Error::new(ErrorKind::PayloadTooLarge, "payload_too_large", "{detail}")
                            .with_param("detail", rejection.body_text())
                    } else {
                        Error::new(ErrorKind::BadRequest, "bad_request", "{detail}")
                            .with_param("detail", rejection.body_text())
                    }
                })?;
        if contains_nul_escape(&bytes) {
            return Err(nul_refused());
        }
        let request = Request::from_parts(parts, axum::body::Body::from(bytes));
        let Json(value) = Json::<T>::from_request(request, state).await.map_err(from_rejection)?;
        value.check()?;
        Ok(ValidJson(value))
    }
}

/// Turns axum's JSON read error into the common error, keeping its description (e.g. "missing field `name`
/// at line 1 column 2") so the client knows what to fix.
fn from_rejection(rejection: JsonRejection) -> Error {
    let detail = rejection.body_text();
    // Lỗi dữ liệu (thiếu trường, sai kiểu, trường lạ): giữ `code` `invalid_body`, chỉ viết lại câu của serde
    // cho người đọc API — nêu tên trường, bỏ câu "Failed to deserialize the JSON body into the target type".
    if matches!(rejection, JsonRejection::JsonDataError(_)) {
        return Error::new(
            ErrorKind::Validation,
            "invalid_body",
            "The request body is invalid: {detail}",
        )
        .with_param("detail", readable_data_error(&detail));
    }
    let (kind, code) = match rejection {
        JsonRejection::JsonSyntaxError(_) => (ErrorKind::BadRequest, "invalid_json"),
        JsonRejection::MissingJsonContentType(_) => {
            (ErrorKind::UnsupportedMediaType, "unsupported_media_type")
        }
        // Còn lại là lỗi đọc thân request: quá lớn (413) hoặc đọc hỏng (400).
        other if other.status().as_u16() == 413 => {
            (ErrorKind::PayloadTooLarge, "payload_too_large")
        }
        _ => (ErrorKind::BadRequest, "bad_request"),
    };
    Error::new(kind, code, "{detail}").with_param("detail", detail)
}

/// The sentence axum puts before serde's own description of a data error.
const DATA_ERROR_PREFIX: &str = "Failed to deserialize the JSON body into the target type: ";

/// Serde's description of a data error, in words an API caller can act on. A missing field is named
/// ("the required field `password` is missing"); anything else keeps serde's text (it already names an unknown
/// field and the fields expected, or the wrong type with its position).
fn readable_data_error(raw: &str) -> String {
    let text = raw.strip_prefix(DATA_ERROR_PREFIX).unwrap_or(raw);
    let missing = text
        .strip_prefix("missing field `")
        .and_then(|rest| rest.split_once('`'))
        .map(|(name, _)| name)
        .filter(|name| !name.is_empty());
    match missing {
        Some(name) => format!("the required field `{name}` is missing."),
        None => text.to_owned(),
    }
}

/// The NUL character (U+0000): Postgres cannot store it in a text column (a 500 error), SQLite can - the
/// two databases differ. The kernel rejects it at every entry point (JSON body, MCP tool arguments, query,
/// path, page cursor) with the same error (D-157).
pub(crate) fn nul_refused() -> Error {
    Error::bad_request(
        "invalid_character",
        "The request contains the NUL character (`\\u0000` in JSON, `%00` in a URL), which no \
         text field accepts. Remove it and send again.",
    )
}

/// Whether the JSON body `bytes` has a `\u0000` escape sequence (a bare byte 0 is not accepted by JSON
/// anyway). Skips `\\u0000` (the `\` is already escaped: plain text, not NUL).
pub(crate) fn contains_nul_escape(bytes: &[u8]) -> bool {
    let mut index = 0;
    while let Some(found) = find(&bytes[index..], b"u0000") {
        let at = index + found;
        let backslashes = bytes[..at].iter().rev().take_while(|byte| **byte == b'\\').count();
        if backslashes % 2 == 1 {
            return true;
        }
        index = at + 1;
    }
    false
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// Whether the JSON `value` has a string (key or value) containing NUL.
pub(crate) fn value_contains_nul(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text.contains('\0'),
        serde_json::Value::Array(items) => items.iter().any(value_contains_nul),
        serde_json::Value::Object(map) => {
            map.iter().any(|(key, item)| key.contains('\0') || value_contains_nul(item))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loi_du_lieu_nen_ten_truong_thieu_khong_lo_cau_cua_serde() {
        let raw = "Failed to deserialize the JSON body into the target type: missing field `password` at line 1 column 13";
        assert_eq!(readable_data_error(raw), "the required field `password` is missing.");
        let unknown = "Failed to deserialize the JSON body into the target type: unknown field `pasword`, expected `email` or `password` at line 1 column 9";
        let text = readable_data_error(unknown);
        assert!(text.starts_with("unknown field `pasword`"), "{text}");
        assert!(!text.contains("Failed to deserialize"), "{text}");
    }

    #[test]
    fn nhan_ra_nul_trong_json() {
        assert!(contains_nul_escape(br#"{"title":"a\u0000b"}"#));
        assert!(contains_nul_escape(br#"{"t":"\\\u0000"}"#));
        // `\\u0000` là dấu `\` rồi chữ `u0000`: không phải NUL.
        assert!(!contains_nul_escape(br#"{"t":"\\u0000"}"#));
        assert!(!contains_nul_escape(br#"{"t":"u0000"}"#));
        assert!(value_contains_nul(&serde_json::json!({"a": ["x", "y\u{0}"]})));
        assert!(value_contains_nul(&serde_json::json!({"k\u{0}": 1})));
        assert!(!value_contains_nul(&serde_json::json!({"a": ["x"], "b": 2})));
    }
}
