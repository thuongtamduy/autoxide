//! Hình dạng của mọi response: envelope cho kết quả, và thân lỗi theo chuẩn.
//!
//! - Thành công, một đối tượng: `{"data": {…}}` — kiểu [`Data`].
//! - Thành công, một danh sách: `{"data": […], "pagination": {…}}` — kiểu
//!   [`Page`](crate::pagination::Page).
//! - Lỗi: thân **RFC 9457 Problem Details** (`application/problem+json`) — kiểu
//!   [`Problem`], thêm hai trường mở rộng `code` và `errors`.
//!
//! Mọi endpoint dùng đúng ba hình dạng này: client (người hay AI agent) học một
//! endpoint là đọc được mọi endpoint.

use std::sync::Arc;

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{Error, ErrorKind};

// Chú ý: doc comment `///` trên kiểu có `ToSchema` (và trên từng trường của nó)
// được chép nguyên vào tài liệu OpenAPI — nên viết tiếng Anh, cho người gọi API
// đọc. Giải thích cho người đọc code thì viết bằng `//` như đoạn này.
//
// `Data { data: 42 }` thành JSON `{"data": 42}`.
/// Envelope of a single result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Data<T> {
    pub data: T,
}

// Handler trả thẳng `Data { data }` (hoặc `(StatusCode::CREATED, Data { data })`):
// axum gọi `into_response` để đổi thành response JSON.
impl<T: Serialize> IntoResponse for Data<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

/// An error, as RFC 9457 Problem Details. Branch on `code`; show `detail` to people.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Problem {
    /// Always `about:blank`: the problem is identified by `code`, not by a URI.
    #[serde(rename = "type")]
    #[schema(example = "about:blank")]
    pub type_uri: String,
    /// The standard phrase for the HTTP status, e.g. `Not Found`.
    #[schema(example = "Unprocessable Entity")]
    pub title: String,
    /// The HTTP status code, repeated in the body.
    #[schema(example = 422)]
    pub status: u16,
    /// Stable machine-readable identifier of the problem. Branch on this, never
    /// on `detail`.
    #[schema(example = "validation_failed")]
    pub code: String,
    /// Human-readable explanation of what went wrong and how to fix it.
    pub detail: String,
    /// One entry per invalid input field. Present only for validation problems.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ProblemField>,
}

/// One invalid field of the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProblemField {
    /// Path to the field: `name`, `address.city`, `items[2].quantity`.
    #[schema(example = "name")]
    pub field: String,
    /// Stable identifier of the kind of problem: `required`, `length`, `range`…
    #[schema(example = "length")]
    pub code: String,
    /// What the value must look like.
    #[schema(example = "must be between 1 and 50 characters long")]
    pub detail: String,
}

/// Kiểu nội dung của thân lỗi theo RFC 9457.
pub const PROBLEM_CONTENT_TYPE: &str = "application/problem+json";

impl From<&Error> for Problem {
    fn from(error: &Error) -> Self {
        let status = status_of(error);
        Problem {
            type_uri: "about:blank".to_owned(),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            code: error.code().to_owned(),
            detail: error.message(),
            errors: error
                .fields()
                .iter()
                .map(|field| ProblemField {
                    field: field.field.clone(),
                    code: field.code.clone(),
                    detail: field.message.clone(),
                })
                .collect(),
        }
    }
}

fn status_of(error: &Error) -> StatusCode {
    // Mã trong `ErrorKind::status` đều hợp lệ; nhánh dự phòng chỉ để không phải
    // `unwrap`.
    StatusCode::from_u16(error.kind().status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

// Handler trả `Result<_, Error>`; axum gọi hàm này để đổi lỗi thành response.
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let problem = Problem::from(&self);
        let mut response = (status_of(&self), Json(problem)).into_response();
        response
            .headers_mut()
            .insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_CONTENT_TYPE));
        if self.kind() == ErrorKind::Unauthenticated {
            // RFC 9110 bắt 401 phải nói cách xác thực; RFC 6750: cách đó là bearer token.
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        // Gửi kèm chính lỗi (có nguyên nhân gốc) trong "extensions" của response:
        // phần này không ra tới client, nhưng middleware ghi log đọc được.
        response.extensions_mut().insert(Arc::new(self));
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::FieldError;

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn data_la_envelope_cua_ket_qua_don() {
        let response = Data { data: serde_json::json!({"id": 1}) }.into_response();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(body_json(response).await, serde_json::json!({"data": {"id": 1}}));
    }

    #[tokio::test]
    async fn loi_401_noi_cach_xac_thuc() {
        let unauthenticated = Error::unauthenticated("auth.missing_token", "Sign in.");
        let forbidden = Error::forbidden("auth.forbidden", "No.");

        let unauthenticated = unauthenticated.into_response();
        let forbidden = forbidden.into_response();

        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(unauthenticated.headers()[header::WWW_AUTHENTICATE], "Bearer");
        assert!(!forbidden.headers().contains_key(header::WWW_AUTHENTICATE));
    }

    #[tokio::test]
    async fn loi_thanh_problem_details() {
        let error = Error::not_found("notes.note_not_found", "Note {id} does not exist.")
            .with_param("id", 7);

        let response = error.into_response();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CONTENT_TYPE], PROBLEM_CONTENT_TYPE);
        assert_eq!(
            body_json(response).await,
            serde_json::json!({
                "type": "about:blank",
                "title": "Not Found",
                "status": 404,
                "code": "notes.note_not_found",
                "detail": "Note 7 does not exist.",
            })
        );
    }

    #[tokio::test]
    async fn loi_dau_vao_co_danh_sach_truong() {
        let error = Error::validation(vec![FieldError::new("name", "length", "must not be empty")]);

        let body = body_json(error.into_response()).await;

        assert_eq!(body["status"], 422);
        assert_eq!(body["title"], "Unprocessable Entity");
        assert_eq!(body["code"], "validation_failed");
        assert_eq!(
            body["errors"],
            serde_json::json!([{"field": "name", "code": "length", "detail": "must not be empty"}])
        );
    }

    #[tokio::test]
    async fn loi_noi_bo_khong_lo_nguyen_nhan_nhung_giu_cho_log() {
        let response =
            Error::internal("password authentication failed for user app").into_response();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        // Middleware ghi log lấy lại được lỗi đầy đủ…
        let logged = response.extensions().get::<Arc<Error>>().unwrap().to_string();
        assert!(logged.contains("password authentication failed"), "{logged}");
        // …còn client thì không thấy.
        let body = body_json(response).await.to_string();
        assert!(body.contains("\"code\":\"internal_error\""), "{body}");
        assert!(!body.contains("password"), "{body}");
    }

    #[test]
    fn problem_doc_lai_duoc_tu_json() {
        let problem = Problem::from(&Error::conflict("c", "m"));
        let json = serde_json::to_string(&problem).unwrap();
        // `errors` rỗng thì không có mặt trong JSON, và đọc lại vẫn được.
        assert!(!json.contains("errors"), "{json}");
        assert_eq!(serde_json::from_str::<Problem>(&json).unwrap(), problem);
    }

    #[derive(Serialize, ToSchema)]
    struct Sample {
        id: u32,
    }

    #[derive(Serialize, ToSchema)]
    struct Other {
        name: String,
    }

    // Chỉ để macro sinh tài liệu; không ai gọi hai hàm này.
    #[utoipa::path(get, path = "/samples/1", responses((status = 200, body = Data<Sample>)))]
    #[allow(dead_code)]
    async fn sample() {}

    #[utoipa::path(
        get,
        path = "/others/1",
        responses((status = 200, body = Data<Other>), (status = 404, body = Problem))
    )]
    #[allow(dead_code)]
    async fn other() {}

    #[derive(utoipa::OpenApi)]
    #[openapi(paths(sample, other))]
    struct Document;

    #[test]
    fn openapi_tach_envelope_theo_kieu_ben_trong() {
        use utoipa::OpenApi;
        // Hai endpoint trả `Data` với hai kiểu dữ liệu khác nhau phải thành hai
        // schema riêng; nếu chung một tên thì tài liệu của endpoint này đè lên
        // endpoint kia.
        let document = serde_json::to_value(Document::openapi()).unwrap();
        let schemas = document["components"]["schemas"].as_object().unwrap();
        let names: Vec<&str> = schemas.keys().map(String::as_str).collect();

        assert_eq!(
            names,
            ["Data_Other", "Data_Sample", "Other", "Problem", "ProblemField", "Sample"]
        );
        assert_eq!(schemas["Data_Sample"]["required"], serde_json::json!(["data"]));
        assert!(schemas["Data_Sample"]["properties"]["data"]["properties"]["id"].is_object());
        // Doc comment `///` của kiểu đi thẳng vào tài liệu API: phải là tiếng Anh.
        assert_eq!(schemas["Data_Sample"]["description"], "Envelope of a single result.");
        let ok = &document["paths"]["/others/1"]["get"]["responses"]["200"];
        assert_eq!(
            ok["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/Data_Other"
        );
    }
}
