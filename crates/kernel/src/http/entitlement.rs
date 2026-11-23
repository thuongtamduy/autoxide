//! Plan-based gating layer for a module's REST routes (step 35, D-325): wraps each module's router when
//! mounted under `/api/v1/<module>`; counts requests for the `kernel.api_requests` quota in every module,
//! and blocks only in modules that are not `always_enabled`.
//!
//! - The tenant's plan lacks the module: 403 `module_not_enabled`;
//! - the tenant is read-only due to its subscription (suspended / cancelled): write requests (`POST`,
//!   `PUT`, `PATCH`, `DELETE`) get 403 `tenant_read_only`; reads still pass.
//!
//! The request's tenant is resolved with the same extractor the handlers use (the result is cached in the
//! request, so handlers do not resolve it again). If it cannot be resolved (a public route without a
//! tenant...), the request passes through: the handler returns its own error.

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::entitlement::{EntitlementsHandle, TenantAccess, module_not_enabled};

pub(crate) async fn gate(
    entitlements: EntitlementsHandle,
    module: &'static str,
    always_enabled: bool,
    request: Request,
    next: Next,
) -> Response {
    if !entitlements.is_enforced() {
        return next.run(request).await;
    }
    let (mut parts, body) = request.into_parts();
    if let Ok(tenant) = crate::tenancy::request::tenant(&mut parts).await {
        // Đếm request theo khách cho hạn mức `kernel.api_requests` (mọi module).
        entitlements.count_request(tenant.id());
        if always_enabled {
            return next.run(Request::from_parts(parts, body)).await;
        }
        // Hết lượng request của kỳ (gói đặt mức tối đa): module nền vẫn chạy (đăng nhập, xem gói).
        if let Some(wait) = entitlements.request_quota_wait(tenant.id()) {
            return crate::entitlement::quota_exhausted(
                crate::entitlement::API_REQUESTS.key(),
                wait,
            )
            .into_response();
        }
        let snapshot = match entitlements.snapshot(tenant.id()).await {
            Ok(snapshot) => snapshot,
            Err(error) => return error.into_response(),
        };
        if !snapshot.has_module(module) {
            metrics::counter!("entitlement_denials_total", "kind" => "module").increment(1);
            return module_not_enabled(module).into_response();
        }
        let writes = !matches!(parts.method, Method::GET | Method::HEAD | Method::OPTIONS);
        if writes && snapshot.access == TenantAccess::ReadOnly {
            metrics::counter!("entitlement_denials_total", "kind" => "read_only").increment(1);
            return crate::tenancy::errors::tenant_read_only().into_response();
        }
    }
    next.run(Request::from_parts(parts, body)).await
}
