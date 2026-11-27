//! Đánh dấu kết quả gửi lại cùng `Idempotency-Key` (UT-6 L3). Đường công khai không có principal nên
//! lớp idempotency của kernel bỏ qua; orders tự chống trùng, nên handler phải tự gắn
//! `Idempotent-Replayed: true` như kernel làm (`docs/guides/retries.md`).

use autoxide_kernel::reference::Submitted;
use axum::http::{HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};

/// Header kernel gắn cho bản lưu (cùng tên, viết thường như mọi `HeaderName`).
const IDEMPOTENT_REPLAYED: HeaderName = HeaderName::from_static("idempotent-replayed");

/// Thân `body` đã thành response; lần gửi lại (`replayed`) mang header `Idempotent-Replayed: true`.
pub(super) fn marked(body: impl IntoResponse, replayed: bool) -> Response {
    let mut response = body.into_response();
    if replayed {
        response.headers_mut().insert(IDEMPOTENT_REPLAYED, HeaderValue::from_static("true"));
    }
    response
}

/// Lần gửi lại hay không, và giá trị bên trong.
pub(super) fn split<T>(submitted: Submitted<T>) -> (bool, T) {
    match submitted {
        Submitted::Created(value) => (false, value),
        Submitted::Replayed(value) => (true, value),
    }
}
