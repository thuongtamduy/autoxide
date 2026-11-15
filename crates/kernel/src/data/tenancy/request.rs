//! Khách của một request (đặc tả 33 "Ngữ cảnh khách cho request").
//!
//! 1. Chế độ `single`: luôn khách mặc định; bỏ qua tên miền và header; không truy vấn gì thêm.
//! 2. Có credential (`Authorization`): khách lấy **từ chính credential** (bộ xác thực của `auth`
//!    đặt [`Principal::in_tenant`]).
//! 3. Header `Autoxide-Tenant: <slug | id>`: nhân sự của khách nền tảng có permission
//!    `kernel.tenants.operate` vào được khách đó ([`Principal::operator_from`]); credential của
//!    khách khác trỏ khách khác → `tenant_mismatch`; không credential → chọn khách cho route công
//!    khai.
//! 4. Tên miền (`Host`) đã xác minh của một khách → khách đó; khác khách của credential →
//!    `tenant_mismatch`.
//! 5. Không xác định được → handler cần [`TenantContext`] nhận `tenant_required`.
//!
//! Kết quả xác thực giữ trong extensions của request: handler nhận cả `Principal` lẫn
//! `TenantContext` chỉ xác thực một lần.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, header};
use uuid::Uuid;

use super::{DatabaseRegistry, TenancyMode, TenantContext, ensure_serving, errors};
use crate::access::{Authenticator, Principal};
use crate::error::Error;

/// Header chọn khách: `Autoxide-Tenant: <slug | id>`.
pub const TENANT_HEADER: HeaderName = HeaderName::from_static("autoxide-tenant");

/// Permission cho nhân sự nền tảng vào khách khác bằng header (khai ở module `kernel`).
pub(crate) const OPERATE: &str = "kernel.tenants.operate";

/// Khách mà request nhắm tới, đọc từ tên miền và header — chưa kiểm gì.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TenantHint {
    host: Option<String>,
    tenant: Option<String>,
}

impl TenantHint {
    /// Không gợi ý gì (lệnh CLI, job, test).
    pub fn none() -> Self {
        TenantHint::default()
    }

    /// Gợi ý từ tên miền `host` (không cổng) và giá trị header `Autoxide-Tenant`.
    pub fn new(host: Option<String>, tenant: Option<String>) -> Self {
        TenantHint {
            host: host.map(|host| normalize_host(&host)).filter(|host| !host.is_empty()),
            tenant: tenant
                .map(|tenant| tenant.trim().to_owned())
                .filter(|tenant| !tenant.is_empty()),
        }
    }

    /// Gợi ý từ header `Host` và `Autoxide-Tenant` của request.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let text = |name: &HeaderName| {
            headers.get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
        };
        TenantHint::new(text(&header::HOST), text(&TENANT_HEADER))
    }

    /// Tên miền (chữ thường, không cổng, không dấu chấm cuối).
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Giá trị header `Autoxide-Tenant`: slug hay ID của khách.
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }
}

/// `Shop.Example.:8443` → `shop.example`.
fn normalize_host(host: &str) -> String {
    let host = host.trim();
    // IPv6 trong ngoặc vuông: giữ nguyên phần trong ngoặc.
    let without_port = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host.rsplit_once(':').map_or(host, |(name, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) { name } else { host }
        })
    };
    without_port.trim_end_matches('.').to_ascii_lowercase()
}

/// Bên gọi đã xác thực, giữ trong extensions để không xác thực hai lần.
#[derive(Clone)]
struct Authenticated(Principal);

/// Xác thực request bằng `authenticator` rồi kiểm khách (chế độ `multi`). Kết quả giữ trong
/// extensions của request.
pub(crate) async fn authenticate(
    authenticator: &Arc<dyn Authenticator>,
    parts: &mut Parts,
) -> Result<Principal, Error> {
    if let Some(Authenticated(principal)) = parts.extensions.get::<Authenticated>() {
        return Ok(principal.clone());
    }
    let hint = TenantHint::from_headers(&parts.headers);
    let authorization =
        parts.headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());
    let principal = authenticator.authenticate(authorization, &hint).await?;
    let principal = match parts.extensions.get::<DatabaseRegistry>() {
        Some(registry) if registry.mode() == TenancyMode::Multi => {
            check(registry, principal, &hint).await?
        }
        _ => principal,
    };
    parts.extensions.insert(Authenticated(principal.clone()));
    Ok(principal)
}

/// Khách của credential khớp với tên miền / header; nhân sự nền tảng vào khách khác.
async fn check(
    registry: &DatabaseRegistry,
    principal: Principal,
    hint: &TenantHint,
) -> Result<Principal, Error> {
    if let Some(target) = hint.tenant() {
        let target = lookup(registry, target).await?;
        if target.id() == principal.tenant_id() {
            return Ok(principal);
        }
        let home = registry.tenant(principal.tenant_id()).await?;
        if home.is_platform() && principal.allows(OPERATE) {
            tracing::info!(
                user_id = %principal.user_id(),
                tenant_id = %target.id(),
                "tenancy entered"
            );
            return Ok(principal.operating(target.id()));
        }
        return Err(errors::tenant_mismatch());
    }
    if let Some(host) = hint.host()
        && let Some(owner) = registry.tenant_by_domain(host).await?
        && owner.id() != principal.tenant_id()
    {
        return Err(errors::tenant_mismatch());
    }
    Ok(principal)
}

/// Khách theo giá trị header: ID (UUID) hay slug.
async fn lookup(registry: &DatabaseRegistry, raw: &str) -> Result<TenantContext, Error> {
    match raw.parse::<Uuid>() {
        Ok(id) => registry.tenant(super::TenantId::from_uuid(id)).await,
        Err(_) => registry.tenant_by_slug(raw).await,
    }
}

/// Khách của request (xem đầu file). Chế độ `single`: khách mặc định, không truy vấn gì.
pub(crate) async fn tenant(parts: &mut Parts) -> Result<TenantContext, Error> {
    if let Some(tenant) = parts.extensions.get::<TenantContext>() {
        return Ok(tenant.clone());
    }
    let registry = parts.extensions.get::<DatabaseRegistry>().cloned().ok_or_else(|| {
        Error::internal(
            "an endpoint needs the tenant of the request, but no tenancy registry is set",
        )
    })?;
    if registry.mode() == TenancyMode::Single {
        let tenant = registry.default_tenant();
        parts.extensions.insert(tenant.clone());
        return Ok(tenant);
    }
    let tenant = if parts.headers.contains_key(header::AUTHORIZATION) {
        let authenticator =
            parts.extensions.get::<Arc<dyn Authenticator>>().cloned().ok_or_else(|| {
                Error::internal(
                    "no module provides `dyn Authenticator`; register the `auth` module",
                )
            })?;
        let principal = authenticate(&authenticator, parts).await?;
        registry.tenant(principal.tenant_id()).await?
    } else {
        let hint = TenantHint::from_headers(&parts.headers);
        match (hint.tenant(), hint.host()) {
            (Some(target), _) => lookup(&registry, target).await?,
            (None, Some(host)) => {
                registry.tenant_by_domain(host).await?.ok_or_else(errors::tenant_required)?
            }
            (None, None) => return Err(errors::tenant_required()),
        }
    };
    ensure_serving(&tenant)?;
    parts.extensions.insert(tenant.clone());
    Ok(tenant)
}

// Handler viết `tenant: TenantContext` thì axum giải khách của request trước khi chạy handler.
impl<S: Send + Sync> FromRequestParts<S> for TenantContext {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // `Box::pin`: như extractor `Principal` — future của handler không phình theo.
        Box::pin(tenant(parts)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_mien_bo_cong_chu_hoa_dau_cham() {
        assert_eq!(normalize_host("Shop.Example.:8443"), "shop.example");
        assert_eq!(normalize_host("shop.example"), "shop.example");
        assert_eq!(normalize_host("[::1]:8080"), "::1");
        let hint = TenantHint::new(Some(" ".to_owned()), Some(" acme ".to_owned()));
        assert_eq!((hint.host(), hint.tenant()), (None, Some("acme")));
        assert_eq!(OPERATE, crate::builtin::TENANTS_OPERATE.name());
    }
}
