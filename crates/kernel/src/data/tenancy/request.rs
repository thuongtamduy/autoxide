//! The tenant of a request (spec 33 "Tenant context for requests").
//!
//! 1. `single` mode: always the default tenant; ignores the domain and header; runs no extra query.
//! 2. With a credential (`Authorization`): the tenant is taken **from the credential itself** (the `auth`
//!    authenticator sets [`Principal::in_tenant`]).
//! 3. Header `Autoxide-Tenant: <slug | id>`: platform-tenant staff with the permission
//!    `kernel.tenants.operate` can enter that tenant ([`Principal::operator_from`]); a credential of one
//!    tenant that points at another tenant → `tenant_mismatch`; with no credential → selects the tenant for
//!    public routes.
//! 4. A verified domain (`Host`) of a tenant → that tenant; different from the credential's tenant →
//!    `tenant_mismatch`.
//! 5. Cannot be determined → a handler that needs [`TenantContext`] gets `tenant_required`.
//!
//! The resolved tenant: suspended / maintenance → error ([`ensure_serving`]); read-only while the request
//! writes (not `GET` / `HEAD` / `OPTIONS`, route not marked `AllowedWhileReadOnly`) → `tenant_read_only`
//! ([`ensure_writable`]). One place checks both flags for every handler that touches tenant data.
//!
//! The authentication result is kept in the request's extensions: a handler that takes both `Principal` and
//! `TenantContext` authenticates only once.

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

/// Header that selects the tenant: `Autoxide-Tenant: <slug | id>`.
pub const TENANT_HEADER: HeaderName = HeaderName::from_static("autoxide-tenant");

/// Permission for platform staff to enter another tenant with the header (declared in the `kernel` module).
pub(crate) const OPERATE: &str = "kernel.tenants.operate";

/// The tenant the request targets, read from the domain and header, with nothing checked yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TenantHint {
    host: Option<String>,
    tenant: Option<String>,
}

impl TenantHint {
    /// No hint (CLI command, job, test).
    pub fn none() -> Self {
        TenantHint::default()
    }

    /// A hint from the domain `host` (no port) and the value of the `Autoxide-Tenant` header.
    pub fn new(host: Option<String>, tenant: Option<String>) -> Self {
        TenantHint {
            host: host.map(|host| normalize_host(&host)).filter(|host| !host.is_empty()),
            tenant: tenant
                .map(|tenant| tenant.trim().to_owned())
                .filter(|tenant| !tenant.is_empty()),
        }
    }

    /// A hint from the request's `Host` and `Autoxide-Tenant` headers.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let text = |name: &HeaderName| {
            headers.get(name).and_then(|value| value.to_str().ok()).map(str::to_owned)
        };
        TenantHint::new(text(&header::HOST), text(&TENANT_HEADER))
    }

    /// The request's hint: the domain a trusted proxy forwards (`X-Forwarded-Host`, layer
    /// `client::attach`) or the `Host` header, and the `Autoxide-Tenant` header.
    pub(crate) fn of_request(headers: &HeaderMap, extensions: &Extensions) -> Self {
        match extensions.get::<crate::http::ForwardedHost>() {
            Some(forwarded) => {
                let tenant = headers.get(&TENANT_HEADER).and_then(|value| value.to_str().ok());
                TenantHint::new(Some(forwarded.0.clone()), tenant.map(str::to_owned))
            }
            None => TenantHint::from_headers(headers),
        }
    }

    /// Domain (lowercase, no port, no trailing dot).
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Value of the `Autoxide-Tenant` header: the tenant's slug or ID.
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

/// The authenticated caller, kept in extensions so authentication does not run twice.
#[derive(Clone)]
pub(crate) struct Authenticated(pub(crate) Principal);

/// Authenticates the request with `authenticator` then checks the tenant (`multi` mode). The result is kept
/// in the request's extensions.
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
    crate::http::RequestSpan::note_tenant(&parts.extensions, principal.tenant_id());
    Ok(principal)
}

/// The credential's tenant matching the domain / header; platform staff entering another tenant.
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

/// The tenant by header value: ID (UUID) or slug.
async fn lookup(registry: &DatabaseRegistry, raw: &str) -> Result<TenantContext, Error> {
    match raw.parse::<Uuid>() {
        Ok(id) => registry.tenant(super::TenantId::from_uuid(id)).await,
        Err(_) => registry.tenant_by_slug(raw).await,
    }
}

/// The request's tenant (see the top of the file). `single` mode: the default tenant, no query.
pub(crate) async fn tenant(parts: &mut Parts) -> Result<TenantContext, Error> {
    if let Some(tenant) = parts.extensions.get::<TenantContext>() {
        return Ok(tenant.clone());
    }
    let tenant = resolve_tenant(parts).await?;
    if let Some(slot) = parts.extensions.get::<crate::ops::request_log::CallerSlot>() {
        slot.note_tenant(&tenant);
    }
    crate::http::RequestSpan::note_tenant(&parts.extensions, tenant.id());
    // Sandbox (D-232): lớp HTTP thêm `Autoxide-Sandbox` và `X-Robots-Tag` vào response.
    if let Some(slot) = parts.extensions.get::<super::sandbox::headers::SandboxSlot>() {
        slot.note(&tenant);
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

/// The tenant by the `autoxide-tenant` header / domain, ignoring the credential (a request that is not signed
/// in).
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

/// The request's tenant for an endpoint that accepts a token that carries the tenant itself
/// (`OptionalTenant`): like [`tenant`], but an invalid credential in `Authorization` (e.g. an expired
/// access token that the client still sends along when calling `/refresh`) is ignored and the tenant is
/// chosen by header / domain. If the tenant cannot be determined: `tenant_required`.
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
