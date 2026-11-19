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
//! Khách đã giải: tạm khoá / bảo trì → lỗi ([`ensure_serving`]); chỉ đọc mà request ghi (không phải
//! `GET` / `HEAD` / `OPTIONS`, route không đánh dấu `AllowedWhileReadOnly`) → `tenant_read_only`
//! ([`ensure_writable`]). Một chỗ kiểm cả hai cờ cho mọi handler chạm dữ liệu của khách.
//!
//! Kết quả xác thực giữ trong extensions của request: handler nhận cả `Principal` lẫn
//! `TenantContext` chỉ xác thực một lần.

use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{Extensions, HeaderMap, HeaderName, header};
use uuid::Uuid;

use super::{
    DatabaseRegistry, TenancyMode, TenantContext, ensure_serving, ensure_writable, errors,
};
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

    /// Gợi ý của request: tên miền proxy tin cậy chuyển tới (`X-Forwarded-Host`, lớp
    /// `client::attach`) hay header `Host`, và header `Autoxide-Tenant`.
    pub(crate) fn of_request(headers: &HeaderMap, extensions: &Extensions) -> Self {
        match extensions.get::<crate::http::ForwardedHost>() {
            Some(forwarded) => {
                let tenant = headers.get(&TENANT_HEADER).and_then(|value| value.to_str().ok());
                TenantHint::new(Some(forwarded.0.clone()), tenant.map(str::to_owned))
            }
            None => TenantHint::from_headers(headers),
        }
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
    let hint = TenantHint::of_request(&parts.headers, &parts.extensions);
    let authorization =
        parts.headers.get(header::AUTHORIZATION).and_then(|value| value.to_str().ok());
    let principal = authenticator.authenticate(authorization, &hint).await?;
    // API key giới hạn dải IP (bước 54): kiểm ở một chỗ cho mọi endpoint REST và MCP.
    let client = parts.extensions.get::<crate::http::ClientIp>().and_then(|ip| ip.0);
    principal.admits(client)?;
    let principal = match parts.extensions.get::<DatabaseRegistry>() {
        Some(registry) if registry.mode() == TenancyMode::Multi => {
            check(registry, authenticator.as_ref(), principal, &hint).await?
        }
        _ => principal,
    };
    parts.extensions.insert(Authenticated(principal.clone()));
    if let Some(slot) = parts.extensions.get::<crate::ops::request_log::CallerSlot>() {
        slot.note_principal(&principal);
    }
    Ok(principal)
}

/// Khách của credential khớp với tên miền / header; nhân sự nền tảng vào khách khác.
async fn check(
    registry: &DatabaseRegistry,
    authenticator: &dyn Authenticator,
    principal: Principal,
    hint: &TenantHint,
) -> Result<Principal, Error> {
    if let Some(target) = hint.tenant() {
        let target = lookup(registry, target).await?;
        if target.id() == principal.tenant_id() {
            return Ok(principal);
        }
        // Đường vào do module xác thực quyết (uỷ quyền, hỗ trợ — bước 34).
        if let Some(entered) = authenticator.enter(&principal, &target).await? {
            tracing::info!(
                user_id = %principal.user_id(),
                tenant_id = %target.id(),
                access = entered.access().as_str(),
                "tenancy entered"
            );
            return Ok(entered);
        }
        let home = registry.tenant(principal.tenant_id()).await?;
        if !authenticator.decides_entry() && home.is_platform() && principal.allows(OPERATE) {
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
    let tenant = resolve_tenant(parts).await?;
    if let Some(slot) = parts.extensions.get::<crate::ops::request_log::CallerSlot>() {
        slot.note_tenant(&tenant);
    }
    Ok(tenant)
}

async fn resolve_tenant(parts: &mut Parts) -> Result<TenantContext, Error> {
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
        by_hint(&registry, parts).await?
    };
    ensure_serving(&tenant)?;
    ensure_writable(&tenant, &parts.method, &parts.extensions)?;
    parts.extensions.insert(tenant.clone());
    Ok(tenant)
}

/// Khách theo header `autoxide-tenant` / tên miền, bỏ qua credential (request không đăng nhập).
async fn by_hint(registry: &DatabaseRegistry, parts: &Parts) -> Result<TenantContext, Error> {
    let hint = TenantHint::of_request(&parts.headers, &parts.extensions);
    match (hint.tenant(), hint.host()) {
        (Some(target), _) => lookup(registry, target).await,
        (None, Some(host)) => {
            registry.tenant_by_domain(host).await?.ok_or_else(errors::tenant_required)
        }
        (None, None) => Err(errors::tenant_required()),
    }
}

/// Khách của request cho endpoint nhận token tự mang khách (`OptionalTenant`): như [`tenant`],
/// nhưng credential trong `Authorization` không hợp lệ (vd access token đã hết hạn mà client vẫn
/// gửi kèm khi gọi `/refresh`) thì bỏ qua nó và chọn khách theo header / tên miền. Không xác định
/// được khách: `tenant_required`.
pub(crate) async fn tenant_ignoring_bad_credential(
    parts: &mut Parts,
) -> Result<TenantContext, Error> {
    match tenant(parts).await {
        Err(error) if error.kind() == crate::error::ErrorKind::Unauthenticated => {
            let registry = parts.extensions.get::<DatabaseRegistry>().cloned().ok_or_else(|| {
                Error::internal(
                    "an endpoint needs the tenant of the request, but no tenancy registry is set",
                )
            })?;
            let tenant = by_hint(&registry, parts).await?;
            ensure_serving(&tenant)?;
            ensure_writable(&tenant, &parts.method, &parts.extensions)?;
            Ok(tenant)
        }
        other => other,
    }
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
