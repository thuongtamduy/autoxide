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
