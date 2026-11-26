//! Plan-based gating layer for a module's REST routes (step 35, D-325): wraps each module's router when
//! mounted under `/api/v1/<module>`; counts requests for the `kernel.api_requests` quota in every module,
//! and blocks only in modules that are not `always_enabled`.
//!
//! - The tenant's plan lacks the module: 403 `module_not_enabled`;
//! - the tenant is read-only due to its subscription (suspended / cancelled): write requests (`POST`,
//!   `PUT`, `PATCH`, `DELETE`) get 403 `tenant_read_only`; reads still pass.
//!
//! Only signed-in callers are counted and held to the quota, against the tenant they act in (see
//! `tenant_rate_limit::budget_tenant`); a request refused by the rate limit per tenant is not counted. The request's tenant
//! for the plan checks is resolved with the same extractor the handlers use (the result is cached in the
//! request, so handlers do not resolve it again). If it cannot be resolved (a public route without a
//! tenant...), the request passes through: the handler returns its own error.

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::entitlement::{EntitlementsHandle, TenantAccess, module_not_enabled};

/// Marks a request already counted for `kernel.api_requests`.
#[derive(Clone, Copy)]
struct Counted;

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
    // Khách chịu hạn mức `kernel.api_requests` của request này: chỉ người gọi đã xác thực, vào khách họ làm
    // việc (của mình hay khách đã uỷ quyền; D-20261126-tenant-rate-limit, D-20261126-api-quota-signed-in).
    // Request ẩn danh nêu tên khách không làm cạn, cũng không bị chặn bởi, lượng của khách đó; nhân sự nền tảng
    // và quyền hỗ trợ cũng vậy, để hỗ trợ được khách đã hết lượng. Lệnh trong một lô ghi mang dấu của lô: lô
    // tính một lần.
    let budget = if parts.extensions.get::<Counted>().is_some() {
        None
    } else {
        match super::tenant_rate_limit::signed_in(&mut parts).await {
            Ok(principal) => principal.as_ref().and_then(super::tenant_rate_limit::budget_tenant),
            Err(error) => return error.into_response(),
        }
    };
    if let Ok(tenant) = crate::tenancy::request::tenant(&mut parts).await
        && !always_enabled
    {
        // Hết lượng request của kỳ (gói đặt mức tối đa): module nền vẫn chạy (đăng nhập, xem gói).
        if let Some(wait) = budget.and_then(|budget| entitlements.request_quota_wait(budget)) {
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
    // Đếm khi request được cho qua (sau giới hạn theo khách ở lớp ngoài và các chốt của gói ở trên): request bị
    // từ chối không tiêu lượng của gói, nên agent retry qua 429 không đốt hạn mức.
    if let Some(budget) = budget {
        entitlements.count_request(budget);
        parts.extensions.insert(Counted);
    }
    next.run(Request::from_parts(parts, body)).await
}
