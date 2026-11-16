//! Lớp chặn theo gói cho route REST của một module (bước 35, D-325): bọc router của từng module
//! không `always_enabled` khi ghép dưới `/api/v1/<module>`.
//!
//! - Gói của khách không có module → 403 `module_not_enabled`;
//! - khách chỉ đọc vì thuê bao (tạm khoá / huỷ) → lệnh ghi (`POST`, `PUT`, `PATCH`, `DELETE`) bị
//!   403 `tenant_read_only`; đọc vẫn qua.
//!
//! Khách của request giải bằng cùng extractor handler dùng (kết quả cất trong request, handler
//! không giải lại). Không giải được (route công khai không có khách…) thì cho qua: handler tự trả
//! lỗi của nó.

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::entitlement::{EntitlementsHandle, TenantAccess, module_not_enabled};

pub(crate) async fn gate(
    entitlements: EntitlementsHandle,
    module: &'static str,
    request: Request,
    next: Next,
) -> Response {
    if !entitlements.is_enforced() {
        return next.run(request).await;
    }
    let (mut parts, body) = request.into_parts();
    if let Ok(tenant) = crate::tenancy::request::tenant(&mut parts).await {
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
