//! Lỗi của endpoint SCIM theo RFC 7644 §3.12: thân `urn:ietf:params:scim:api:messages:2.0:Error`
//! (`status` là CHUỖI, `scimType` khi có), content-type `application/scim+json`. Khác envelope
//! Problem của Autoxide có chủ đích: client SCIM (Entra ID, Okta) chỉ đọc dạng này
//! (D-20261121-scim). Response mang `StandardErrorBody` để lớp chuẩn hoá lỗi của kernel không đổi
//! nó thành Problem.

use autoxide_kernel::error::{Error, ErrorKind};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Content-type của mọi response SCIM (RFC 7644 §3.1).
pub(crate) const SCIM_CONTENT_TYPE: &str = "application/scim+json";
/// Lược đồ của thân lỗi.
pub(crate) const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";

/// `scimType` của RFC 7644 bảng 9 mà Autoxide dùng.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScimType {
    InvalidFilter,
    Uniqueness,
    Mutability,
    InvalidSyntax,
    InvalidPath,
    NoTarget,
    InvalidValue,
}

impl ScimType {
    fn as_str(self) -> &'static str {
        match self {
            ScimType::InvalidFilter => "invalidFilter",
            ScimType::Uniqueness => "uniqueness",
            ScimType::Mutability => "mutability",
            ScimType::InvalidSyntax => "invalidSyntax",
            ScimType::InvalidPath => "invalidPath",
            ScimType::NoTarget => "noTarget",
            ScimType::InvalidValue => "invalidValue",
        }
    }
}

/// Một lỗi SCIM: mã trạng thái, `scimType` (khi chuẩn có loại phù hợp) và `detail` cho người đọc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScimError {
    pub status: StatusCode,
    pub scim_type: Option<ScimType>,
    pub detail: String,
}

impl ScimError {
    pub(crate) fn new(
        status: StatusCode,
        scim_type: Option<ScimType>,
        detail: impl Into<String>,
    ) -> Self {
        ScimError { status, scim_type, detail: detail.into() }
    }

    /// 400 với `scimType`.
    pub(crate) fn bad(scim_type: ScimType, detail: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, Some(scim_type), detail)
    }

    pub(crate) fn invalid_filter(detail: impl Into<String>) -> Self {
        Self::bad(ScimType::InvalidFilter, detail)
    }

    pub(crate) fn invalid_value(detail: impl Into<String>) -> Self {
        Self::bad(ScimType::InvalidValue, detail)
    }

    pub(crate) fn uniqueness(detail: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, Some(ScimType::Uniqueness), detail)
    }

    pub(crate) fn not_found(kind: &str, id: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, None, format!("{kind} {id} not found"))
    }

    pub(crate) fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            None,
            "Send a SCIM token of this tenant in `Authorization: Bearer axscim1_…`; create one with \
             `POST /api/v1/auth/scim/tokens`.",
        )
    }

    /// Thân JSON của lỗi.
    pub(crate) fn body(&self) -> serde_json::Value {
        let mut body = json!({
            "schemas": [ERROR_SCHEMA],
            "status": self.status.as_u16().to_string(),
            "detail": self.detail,
        });
        if let Some(scim_type) = self.scim_type {
            body["scimType"] = json!(scim_type.as_str());
        }
        body
    }
}

/// Lỗi của service chung (kiểu `Error` của kernel) đổi sang lỗi SCIM theo loại.
impl From<Error> for ScimError {
    fn from(error: Error) -> Self {
        let detail = match error.fields() {
            [] => error.message(),
            fields => fields
                .iter()
                .map(|field| format!("{}: {}", field.field, field.message))
                .collect::<Vec<_>>()
                .join("; "),
        };
        match error.kind() {
            ErrorKind::Validation | ErrorKind::BadRequest => {
                ScimError::bad(ScimType::InvalidValue, detail)
            }
            ErrorKind::Conflict if error.code().ends_with("_taken") => {
                ScimError::uniqueness(detail)
            }
            ErrorKind::Internal => {
                // Chi tiết nội bộ không ra ngoài; ghi log để người vận hành tra.
                tracing::error!(%error, "SCIM request failed");
                ScimError::new(StatusCode::INTERNAL_SERVER_ERROR, None, "Internal error.")
            }
            kind => ScimError::new(
                StatusCode::from_u16(kind.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                None,
                detail,
            ),
        }
    }
}

impl From<sea_orm::DbErr> for ScimError {
    fn from(error: sea_orm::DbErr) -> Self {
        ScimError::from(Error::from(error))
    }
}

impl IntoResponse for ScimError {
    fn into_response(self) -> Response {
        let body = self.body().to_string();
        let mut response = (self.status, body).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(SCIM_CONTENT_TYPE));
        response.extensions_mut().insert(autoxide_kernel::http::StandardErrorBody);
        response
    }
}
