//! CORS: which browser origins may call the API (D-20261125-tenant-cors).
//!
//! An origin is allowed when it is in `AUTOXIDE_HTTP_CORS_ORIGINS` (or that setting is `*`), or - in
//! multi-tenant mode - when it is `https://<domain>` for a domain a tenant has **verified**. The layer
//! [`classify`] decides once per request and leaves an [`OriginGrant`] in the request's extensions; the CORS
//! headers (`tower-http`), the MCP `Origin` check and the tenant check (`tenancy::request`) all read that one
//! answer.
//!
//! - A preflight (`OPTIONS`) carries no credential, so it only learns that the origin belongs to *some*
//!   tenant. The real request is bound: from the origin of tenant A, a request whose tenant (credential,
//!   `Autoxide-Tenant`, `Host`) is B is refused with `tenant_origin_mismatch` before the handler runs.
//! - Matching is exact: the browser serializes `Origin` as lowercase scheme and host, punycode, no default
//!   port, no trailing dot. A tenant origin must be exactly `https://` + a verified domain; anything else
//!   (`null`, `http`, a port, uppercase, trailing dot, Unicode, a path) is refused, never reflected.
//! - The answer comes from a snapshot of every verified domain held by the database registry
//!   (`DatabaseRegistry::verified_domain_owner`): an unknown origin is a map lookup, never a query. The
//!   snapshot is re-read at most once per `AUTOXIDE_TENANCY_CACHE_SECONDS` (default 10 s) and dropped at once
//!   on this server when a domain is verified, revoked, fails or moves to another tenant; other servers follow
//!   within that TTL.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::Response;
use tower_http::cors::{AllowOrigin, CorsLayer};

use super::{REQUEST_ID_HEADER, idempotency};
use crate::config::CorsOrigins;
use crate::tenancy::{DatabaseRegistry, TenancyMode, TenantId, normalize_domain};

/// Why the request's `Origin` is allowed. Absent from the extensions: no `Origin`, or not allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginGrant {
    /// Listed in `AUTOXIDE_HTTP_CORS_ORIGINS` (or that setting is `*`): not bound to a tenant.
    Listed,
    /// `https://` + a domain this tenant verified: the request may only act as this tenant.
    Tenant(TenantId),
}

/// Decides which origins are allowed. Cheap to share (`Arc`).
pub(crate) struct CorsPolicy {
    origins: CorsOrigins,
    /// Only in multi-tenant mode: where tenants' verified domains are looked up.
    registry: Option<DatabaseRegistry>,
}

impl CorsPolicy {
    pub(crate) fn new(origins: CorsOrigins, registry: Option<DatabaseRegistry>) -> Self {
        let registry = registry.filter(|registry| registry.mode() == TenancyMode::Multi);
        CorsPolicy { origins, registry }
    }

    /// Whether any origin can ever be allowed (when not, the CORS layer adds no header at all).
    fn allows_some(&self) -> bool {
        self.registry.is_some() || !matches!(self.origins, CorsOrigins::None)
    }

    /// The grant of `origin`, or `None` when it is not allowed.
    pub(crate) async fn grant(&self, origin: &HeaderValue) -> Option<OriginGrant> {
        match &self.origins {
            CorsOrigins::Any => return Some(OriginGrant::Listed),
            CorsOrigins::List(list)
                if list.iter().any(|allowed| allowed.as_bytes() == origin.as_bytes()) =>
            {
                return Some(OriginGrant::Listed);
            }
            _ => {}
        }
        let registry = self.registry.as_ref()?;
        let domain = tenant_origin_domain(origin.to_str().ok()?)?;
        match registry.verified_domain_owner(domain).await {
            Ok(owner) => owner.map(OriginGrant::Tenant),
            Err(error) => {
                // Không đọc được: từ chối (an toàn hơn cho qua); trình duyệt thử lại sau.
                tracing::warn!(%error, "the verified tenant domains could not be read for CORS");
                None
            }
        }
    }
}

/// The domain of a tenant origin: `origin` must be exactly `https://<domain>` where `<domain>` is already
/// in the normalized form a browser sends (lowercase ASCII / punycode, no port, no trailing dot).
pub(crate) fn tenant_origin_domain(origin: &str) -> Option<&str> {
    let host = origin.strip_prefix("https://")?;
    // `normalize_domain` bỏ port, dấu chấm cuối và hạ chữ hoa: chỉ nhận khi nó không phải đổi gì — so khớp
    // chính xác, không phải "gần đúng".
    (normalize_domain(host).as_deref() == Some(host)).then_some(host)
}

/// Layer: classifies the request's `Origin` once and leaves the [`OriginGrant`] in its extensions. Any
/// grant a client tried to smuggle in is removed first (extensions never come from the network, but this
/// layer is the only one allowed to set it).
pub(crate) async fn classify(
    policy: Arc<CorsPolicy>,
    mut request: Request,
    next: Next,
) -> Response {
    request.extensions_mut().remove::<OriginGrant>();
    let origin = request.headers().get(header::ORIGIN).cloned();
    if let Some(origin) = origin
        && let Some(grant) = policy.grant(&origin).await
    {
        request.extensions_mut().insert(grant);
    }
    next.run(request).await
}

/// The CORS layer: allows the origins [`classify`] granted. `*` stays a literal `*` (no credentials are
/// ever allowed, so a wildcard and an echoed origin give the same access).
pub(crate) fn layer(policy: &CorsPolicy) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::ACCEPT,
            header::ACCEPT_LANGUAGE,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::IF_MATCH,
            header::IF_NONE_MATCH,
            header::IF_RANGE,
            header::RANGE,
            idempotency::IDEMPOTENCY_KEY,
            REQUEST_ID_HEADER,
            crate::tenancy::TENANT_HEADER,
            // Header của giao thức MCP (endpoint `/mcp`).
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("last-event-id"),
        ])
        // Header trình duyệt mặc định giấu khỏi JavaScript mà client cần đọc.
        .expose_headers([
            header::ETAG,
            header::LOCATION,
            header::CONTENT_DISPOSITION,
            header::CONTENT_RANGE,
            header::ACCEPT_RANGES,
            header::RETRY_AFTER,
            super::rate_limit_headers::RATE_LIMIT,
            super::rate_limit_headers::RATE_LIMIT_POLICY,
            header::LINK,
            HeaderName::from_static("deprecation"),
            HeaderName::from_static("sunset"),
            REQUEST_ID_HEADER,
            idempotency::IDEMPOTENT_REPLAYED,
            HeaderName::from_static("mcp-session-id"),
        ])
        .max_age(Duration::from_secs(3600));
    if matches!(policy.origins, CorsOrigins::Any) {
        return layer.allow_origin(AllowOrigin::any());
    }
    if !policy.allows_some() {
        // Không khai origin nào và không có tên miền của khách: không thêm header CORS, trình duyệt tự chặn.
        return layer;
    }
    // Predicate chỉ đọc kết quả `classify` đã để lại: không phản chiếu origin nào chưa được cho phép.
    // Predicate (khác hằng số) làm `tower-http` thêm `Vary: origin`, để cache dùng chung không trả header
    // CORS của origin này cho origin khác.
    layer.allow_origin(AllowOrigin::predicate(|_, parts| {
        parts.extensions.get::<OriginGrant>().is_some()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_cua_khach_phai_dung_dang_chinh_tac() {
        assert_eq!(tenant_origin_domain("https://shop.example"), Some("shop.example"));
        assert_eq!(
            tenant_origin_domain("https://xn--bcher-kva.example"),
            Some("xn--bcher-kva.example")
        );
        for origin in [
            "null",
            "http://shop.example",
            "https://Shop.example",
            "https://shop.example.",
            "https://shop.example:443",
            "https://shop.example:8443",
            "https://shop.example/",
            "https://shop.example/path",
            "https://bücher.example",
            "https://user@shop.example",
            "https://localhost",
            "https://10.0.0.1",
            "HTTPS://shop.example",
            "https://",
            "shop.example",
        ] {
            assert_eq!(tenant_origin_domain(origin), None, "{origin}");
        }
    }
}
