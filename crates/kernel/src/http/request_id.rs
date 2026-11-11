//! ID của request: header `x-request-id`.
//!
//! Dùng để lần một request qua log và để người dùng báo lỗi ("request id là …").

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use tower_http::request_id::{MakeRequestId, RequestId};
use uuid::Uuid;

/// Tên header mang ID của request, ở cả request lẫn response.
pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// ID do client gửi dài hơn mức này thì bị bỏ.
const MAX_CLIENT_ID_LENGTH: usize = 128;

/// Sinh ID cho request chưa có: UUIDv7, nên ID sắp được theo thời gian.
#[derive(Debug, Clone, Copy)]
pub(super) struct MakeUuidV7;

impl MakeRequestId for MakeUuidV7 {
    fn make_request_id<B>(&mut self, _request: &axum::http::Request<B>) -> Option<RequestId> {
        HeaderValue::from_str(&Uuid::now_v7().to_string()).ok().map(RequestId::new)
    }
}

/// Bỏ `x-request-id` do client gửi nếu nó không đáng tin: quá dài, hoặc chứa ký tự
/// ngoài chữ, số và `-_.:`. ID này đi vào log; không để client nhét nội dung tuỳ ý.
/// Lớp kế tiếp sẽ sinh ID mới cho request đó.
pub(super) async fn drop_untrusted(mut request: Request, next: Next) -> Response {
    let trusted = request.headers().get(&REQUEST_ID_HEADER).is_none_or(|value| {
        let bytes = value.as_bytes();
        !bytes.is_empty()
            && bytes.len() <= MAX_CLIENT_ID_LENGTH
            && bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(byte))
    });
    if !trusted {
        request.headers_mut().remove(&REQUEST_ID_HEADER);
    }
    next.run(request).await
}
