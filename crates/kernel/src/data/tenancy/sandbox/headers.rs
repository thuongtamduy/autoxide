//! Headers of every sandbox response (D-232): `Autoxide-Sandbox: true` tells machines they are on a
//! test copy, `X-Robots-Tag: noindex, nofollow` keeps search engines from listing a test website.
//!
//! The HTTP layer puts a cell into the request's extensions; the place that resolves the request's tenant
//! (`request.rs`) marks the cell when the tenant is a sandbox; the layer reads the cell when the response
//! goes out. A request that resolves no tenant (docs page, health) carries no header.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

use crate::tenancy::TenantContext;

/// Name of the header that marks a sandbox response.
pub const SANDBOX_HEADER: HeaderName = HeaderName::from_static("autoxide-sandbox");

/// The cell "this request belongs to a sandbox" (cheap to clone, shared between the HTTP layer and the
/// extractor).
#[derive(Clone, Default)]
pub(crate) struct SandboxSlot(Arc<AtomicBool>);

impl SandboxSlot {
    pub(crate) fn note(&self, tenant: &TenantContext) {
        if tenant.is_sandbox() {
            self.0.store(true, Ordering::Relaxed);
        }
    }
}

/// HTTP layer: adds the two headers when the request's tenant is a sandbox.
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
