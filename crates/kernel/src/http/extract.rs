//! Lấy dữ liệu vào của handler.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use serde::de::DeserializeOwned;
use validator::Validate;

use crate::error::{Error, ErrorKind};

/// Thân JSON của request, đã đọc thành `T` VÀ đã qua `T::validate()`.
///
/// Handler nhận `ValidJson(input): ValidJson<CreateNote>` thì `input` chắc chắn
/// hợp lệ; request sai bị trả lỗi trước khi handler chạy:
///
/// - không phải JSON, hoặc JSON hỏng → 400 `invalid_json`;
/// - thiếu `content-type: application/json` → 415 `unsupported_media_type`;
/// - thiếu trường, sai kiểu → 422 `invalid_body`;
/// - giá trị không qua kiểm tra → 422 `validation_failed` kèm lỗi từng trường.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidJson<T>(pub T);

impl<S, T> FromRequest<S> for ValidJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Validate,
{
    type Rejection = Error;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        // Đọc thân một lần để từ chối ký tự NUL trước khi đọc thành `T` (D-156); rồi để `Json`
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
        value.validate()?;
        Ok(ValidJson(value))
    }
}

/// Đổi lỗi đọc JSON của axum thành lỗi chung, giữ câu mô tả của nó (vd "missing
/// field `name` at line 1 column 2") để client biết sửa gì.
fn from_rejection(rejection: JsonRejection) -> Error {
    let detail = rejection.body_text();
    let (kind, code) = match rejection {
        JsonRejection::JsonDataError(_) => (ErrorKind::Validation, "invalid_body"),
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

/// Ký tự NUL (U+0000): Postgres không lưu được trong cột chữ (lỗi 500), SQLite thì lưu — hai
/// database khác nhau. Kernel từ chối nó ở mọi cửa vào (thân JSON, tham số tool MCP, query,
/// đường dẫn, con trỏ trang) bằng cùng một lỗi (D-156).
pub(crate) fn nul_refused() -> Error {
    Error::bad_request(
        "invalid_character",
        "The request contains the NUL character (`\\u0000` in JSON, `%00` in a URL), which no \
         text field accepts. Remove it and send again.",
    )
}

/// Thân JSON `bytes` có chuỗi thoát `\u0000` không (byte 0 trần thì JSON vốn không nhận). Bỏ
/// qua `\\u0000` (dấu `\` đã được thoát: chữ thường, không phải NUL).
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

/// Giá trị JSON `value` có chuỗi (khoá hay giá trị) chứa NUL không.
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
