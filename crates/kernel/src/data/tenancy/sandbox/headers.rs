//! Header của mọi response của sandbox (D-232): `Autoxide-Sandbox: true` cho máy biết đang ở bản
//! thử, `X-Robots-Tag: noindex, nofollow` để máy tìm kiếm không đưa website thử vào kết quả.
//!
//! Lớp HTTP đặt một ô vào extensions của request; chỗ giải khách của request (`request.rs`) đánh
//! dấu ô khi khách là sandbox; lớp đọc ô khi response quay ra. Request không giải khách (trang tài
//! liệu, health) không mang header.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::tenancy::TenantContext;

/// Tên header đánh dấu response của sandbox.
pub const SANDBOX_HEADER: HeaderName = HeaderName::from_static("autoxide-sandbox");

/// Ô "request này của sandbox" (clone rẻ, chung giữa lớp HTTP và extractor).
#[derive(Clone, Default)]
pub(crate) struct SandboxSlot(Arc<AtomicBool>);

impl SandboxSlot {
    pub(crate) fn note(&self, tenant: &TenantContext) {
        if tenant.is_sandbox() {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}

/// Lớp HTTP: thêm hai header khi khách của request là sandbox.
pub(crate) async fn mark(mut request: Request, next: Next) -> Response {
    let slot = SandboxSlot::default();
    request.extensions_mut().insert(slot.clone());
    let mut response = next.run(request).await;
    if slot.0.load(Ordering::Relaxed) {
        let headers = response.headers_mut();
        headers.insert(SANDBOX_HEADER, HeaderValue::from_static("true"));
        headers.insert(
            HeaderName::from_static("x-robots-tag"),
            HeaderValue::from_static("noindex, nofollow"),
        );
    }
    response
}
