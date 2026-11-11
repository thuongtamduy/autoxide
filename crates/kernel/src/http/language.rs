//! Lớp HTTP chọn ngôn ngữ: đọc `Accept-Language`, dịch `detail` của lỗi, ghi
//! `Content-Language` (D-057).

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

use crate::error::Error;
use crate::i18n::{Language, Translations};
use crate::response::Problem;

pub(super) async fn localize(
    translations: Arc<Translations>,
    request: Request,
    next: Next,
) -> Response {
    let language = Language::negotiate(
        request.headers().get(header::ACCEPT_LANGUAGE).and_then(|value| value.to_str().ok()),
    );
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CONTENT_LANGUAGE, HeaderValue::from_static(language.tag()));
    if language == Language::En {
        return response;
    }
    // Lỗi nào cũng mang chính `Error` trong extensions (do `IntoResponse` của `Error`,
    // hay do lớp chuẩn hoá lỗi gắn vào): dựng lại thân Problem với `detail` đã dịch.
    let Some(error) = response.extensions().get::<Arc<Error>>().cloned() else {
        return response;
    };
    let status: StatusCode = response.status();
    let mut problem = Problem::from(error.as_ref());
    problem.status = status.as_u16();
    problem.title = status.canonical_reason().unwrap_or("Error").to_owned();
    problem.detail = translations.message(&error, language);
    let Ok(body) = serde_json::to_vec(&problem) else {
        return response;
    };
    let (mut parts, _) = response.into_parts();
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(body))
}
