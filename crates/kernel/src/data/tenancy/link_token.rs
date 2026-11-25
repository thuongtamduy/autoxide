//! A token in an email link (unsubscribe, confirm, verify email...) carries its tenant (step 33,
//! D-281). One way for every module that sends links with tokens: `newsletter`, `comments`, later `auth`.
//!
//! The recipient clicks the link from a mailbox: the request may arrive at the main domain, with no
//! `Autoxide-Tenant` header, so the tenant is unknown. So a token of a tenant other than the default writes
//! the tenant ID into the string itself: `t<32-hex UUID>.<raw part>` ([`link_token`]). The endpoint finds
//! the tenant from the token ([`resolve_link_token`]) BEFORE looking up the hash of the raw part in that
//! tenant's database. The token of the default tenant keeps the old form (the raw part only): a
//! single-tenant installation and emails already sent do not change.
//!
//! The database stores only the hash of the **raw part**, looked up in the routed tenant. Combining another
//! tenant's ID with this tenant's raw part looks it up in the other tenant and finds nothing, like a wrong
//! token. The raw part must not contain a `.` (e.g. base64url), so a `.` means there is a tenant prefix.
//!
//! A handler that accepts a token from a link uses [`OptionalTenant`] instead of `TenantContext`: the request's
//! tenant if it can be determined, `None` when not (main domain, no header), instead of
//! `tenant_required`.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use uuid::Uuid;

use super::{DEFAULT_TENANT, DatabaseRegistry, TenantContext, TenantId, ensure_serving};
use crate::error::{Error, ErrorKind};

/// A token longer than this counts as wrong (nobody issues a token that long; nothing to look up).
const MAX_TOKEN_LEN: usize = 200;

/// The token to put in a link for tenant `tenant`, from the raw part `raw` (containing no `.`): the default
/// tenant keeps `raw` as is, other tenants become `t<tenant ID, 32 hex>.<raw>`.
pub fn link_token(tenant: &TenantContext, raw: &str) -> String {
    if tenant.id() == DEFAULT_TENANT {
        raw.to_owned()
    } else {
        format!("t{}.{raw}", tenant.id().as_uuid().simple())
    }
}

/// Splits a link's token into: (the tenant written in the token, if any; the raw part). Wrong form: `None`.
pub fn split_link_token(token: &str) -> Option<(Option<TenantId>, &str)> {
    let Some((prefix, raw)) = token.split_once('.') else {
        return Some((None, token));
    };
    let hex = prefix.strip_prefix('t').filter(|hex| hex.len() == 32)?;
    let id = Uuid::try_parse(hex).ok()?;
    (!raw.is_empty() && !raw.contains('.')).then_some((Some(TenantId::from_uuid(id)), raw))
}

/// The tenant and raw part of a token in a link. `request`: the request's tenant (`None`: cannot be determined,
/// e.g. main domain with no header).
///
/// - Token with a prefix: the tenant written in the token, looked up through `registry`. A tenant that does
///   not exist, cannot be served (suspended, maintenance), a tenant other than the request's when the request
///   came from a page on the request tenant's own domain (CORS), or a malformed token → `Ok(None)`, like a
///   wrong token: it does not reveal which tenant exists.
/// - Token without a prefix: the request's tenant, else the default tenant (tokens issued before step 33
///   belong to the default tenant).
///
/// Other errors of the database registry (e.g. the control database not answering) are returned as they are.
pub async fn resolve_link_token<'t>(
    registry: &DatabaseRegistry,
    request: Option<&TenantContext>,
    token: &'t str,
) -> Result<Option<(TenantContext, &'t str)>, Error> {
    let token = token.trim();
    if token.is_empty() || token.len() > MAX_TOKEN_LEN {
        return Ok(None);
    }
    let Some((id, raw)) = split_link_token(token) else {
        return Ok(None);
    };
    let tenant = match id {
        None => request.cloned().unwrap_or_else(|| registry.default_tenant()),
        // Request đến từ trang trên tên miền của khách khác (CORS): như token sai. Khách đến từ header, `Host` hay
        // credential thì token vẫn thắng — vd người đang đăng nhập ở khách X mở link huỷ nhận thư của khách Y.
        Some(id)
            if request
                .is_some_and(|request| request.is_from_page_origin() && request.id() != id) =>
        {
            return Ok(None);
        }
        Some(id) => match registry.tenant(id).await {
            Ok(tenant) if ensure_serving(&tenant).is_ok() => tenant,
            Ok(_) => return Ok(None),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        },
    };
    Ok(Some((tenant, raw)))
}

/// The request's tenant if it can be determined (the tenant's domain, header `Autoxide-Tenant`,
/// credential); `None` when it cannot be determined (`tenant_required`, e.g. main domain with no header).
/// Other errors (tenant suspended, header pointing to a tenant other than the credential's...) are still
/// returned as for `TenantContext`. For endpoints that accept a token in a link: the tenant is taken from
/// the token with [`resolve_link_token`].
#[derive(Clone, Debug, Default)]
pub struct OptionalTenant(pub Option<TenantContext>);

impl OptionalTenant {
    /// The request's tenant, if it can be determined.
    pub fn get(&self) -> Option<&TenantContext> {
        self.0.as_ref()
    }
}

impl<S: Send + Sync> FromRequestParts<S> for OptionalTenant {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Credential hỏng trong `Authorization` không chặn endpoint đổi token: token tự nêu khách.
        match Box::pin(super::request::tenant_ignoring_bad_credential(parts)).await {
            Ok(tenant) => Ok(OptionalTenant(Some(tenant))),
            Err(error) if error.code() == "tenant_required" => Ok(OptionalTenant(None)),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::tenancy::{MAIN_PLACEMENT, TenantStatus};

    fn other() -> TenantContext {
        TenantContext::new(
            TenantId::from_uuid(Uuid::now_v7()),
            "other",
            "Other",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            Database::detached(),
        )
    }

    #[test]
    fn tach_token_co_va_khong_tien_to_khach() {
        let id = Uuid::now_v7();
        let prefixed = format!("t{}.abc_-9", id.simple());
        assert_eq!(split_link_token(&prefixed), Some((Some(TenantId::from_uuid(id)), "abc_-9")));
        assert_eq!(split_link_token("abc_-9"), Some((None, "abc_-9")));
        for bad in ["x.abc", "t123.abc", &format!("t{}.", id.simple()), &format!("{prefixed}.x")] {
            assert_eq!(split_link_token(bad), None, "{bad}");
        }
    }

    #[test]
    fn khach_mac_dinh_giu_token_tho_khach_khac_co_tien_to() {
        let default = TenantContext::default_on(Database::detached());
        assert_eq!(link_token(&default, "raw"), "raw");
        let other = other();
        let token = link_token(&other, "raw");
        assert_eq!(token, format!("t{}.raw", other.id().as_uuid().simple()));
        assert_eq!(split_link_token(&token), Some((Some(other.id()), "raw")));
    }

    #[tokio::test]
    async fn tim_khach_tu_token() {
        // Sổ `single` không truy vấn gì: khách mặc định có, khách khác `tenant_not_found`.
        let registry = DatabaseRegistry::single(Database::detached());
        let other = other();
        // Không tiền tố: khách của request, không có thì khách mặc định.
        let (tenant, raw) = resolve_link_token(&registry, None, " raw ").await.unwrap().unwrap();
        assert_eq!((tenant.id(), raw), (DEFAULT_TENANT, "raw"));
        let (tenant, _) =
            resolve_link_token(&registry, Some(&other), "raw").await.unwrap().unwrap();
        assert_eq!(tenant, other);
        // Tiền tố của khách mặc định: tra được dù request là khách khác (header, `Host`, credential)…
        let token = format!("t{}.raw", DEFAULT_TENANT.as_uuid().simple());
        let (tenant, raw) =
            resolve_link_token(&registry, Some(&other), &token).await.unwrap().unwrap();
        assert_eq!((tenant.id(), raw), (DEFAULT_TENANT, "raw"));
        // …trừ khi request đến từ trang trên tên miền của khách khác đó (D-20261125-tenant-cors).
        let page = other.clone().from_page_origin();
        assert!(resolve_link_token(&registry, Some(&page), &token).await.unwrap().is_none());
        // Khách không có, sai dạng, rỗng, quá dài: như token sai.
        let forged = link_token(&other, "raw");
        let long = "a".repeat(MAX_TOKEN_LEN + 1);
        for bad in [forged.as_str(), "x.raw", "", "  ", long.as_str()] {
            assert!(resolve_link_token(&registry, None, bad).await.unwrap().is_none(), "{bad}");
        }
    }
}
