//! Links in mail, per tenant (D-20261125-tenant-link-templates).
//!
//! Two layers, the second wins:
//!
//! 1. `kernel.public_url`: the head (scheme + host) of every server-configured link is replaced, the path kept
//!    ([`SiteLinks::rewrite`]).
//! 2. A link setting ([`TenantSetting::link`], group [`LINK_GROUP`], e.g. `auth.link.password_reset`): the
//!    tenant gives the whole address, path included (`https://shop.example/dat-lai-mat-khau?t={token}`)
//!    ([`SiteLinks::link`]). A tenant may enable a mail link the server has not configured.
//!
//! A link setting is untrusted input (any tenant administrator sets it), so it is accepted only when the
//! address is `https`, has no user name, no placeholder in the host part, every placeholder the module fills,
//! and its host is one the tenant is known to control: the host of its `kernel.public_url`, one of its
//! verified domains, the platform host (`AUTOXIDE_HTTP_PUBLIC_URL`) or the host of the server's own link.
//! The check runs when the value is set (422) and again on every use: a domain revoked later makes the link
//! fall back to the server's link instead of sending mail to a host the tenant no longer controls.

use std::sync::Arc;

use super::{PUBLIC_URL, TenantSetting, TenantSettings, parsed_setting, site_origin};
use crate::error::{Error, FieldError};
use crate::tenancy::{TenantContext, TenantDomains};

/// Group of the per-tenant link settings ([`TenantSetting::link`]).
pub const LINK_GROUP: &str = "link";

/// Longest accepted link template.
const LINK_MAX: usize = 2000;

/// What a tenant's link host is checked against: its verified domains and the platform host.
#[derive(Clone, Default)]
pub(crate) struct LinkTrust {
    domains: Option<Arc<TenantDomains>>,
    platform_host: Option<String>,
}

impl LinkTrust {
    pub(crate) fn new(domains: Option<Arc<TenantDomains>>, platform_host: Option<String>) -> Self {
        LinkTrust { domains, platform_host }
    }

    /// Whether `host` belongs to tenant `tenant` (see the module documentation); `server_host`: the host of
    /// the server's own link, when known.
    pub(crate) async fn trusts(
        &self,
        settings: &dyn TenantSettings,
        tenant: &TenantContext,
        host: &str,
        server_host: Option<&str>,
    ) -> Result<bool, Error> {
        if self.platform_host.as_deref() == Some(host) || server_host == Some(host) {
            return Ok(true);
        }
        let site = parsed_setting(settings, tenant, &PUBLIC_URL, |raw| {
            site_origin(raw).map(|()| raw.trim().to_owned())
        })
        .await?;
        if site.as_deref().and_then(parse).is_some_and(|url| host_of(&url) == host) {
            return Ok(true);
        }
        match &self.domains {
            // Truy vấn database điều khiển, chỉ khi khách có đặt link riêng (gửi mail, không phải đường nóng).
            Some(domains) => Ok(domains.verified_hosts(tenant).await?.iter().any(|d| d == host)),
            None => Ok(false),
        }
    }
}

// URL http(s) đọc như trình duyệt (crate `url`, WHATWG) sau khi thay mọi `{…}` bằng `x`; `None` nếu sai dạng.
fn parse(raw: &str) -> Option<url::Url> {
    let filled = fill_placeholders(raw);
    url::Url::parse(&filled).ok().filter(|url| matches!(url.scheme(), "http" | "https"))
}

fn fill_placeholders(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        match rest[start..].find('}') {
            Some(end) => {
                out.push('x');
                rest = &rest[start + end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

// Tên host chữ thường, bỏ dấu chấm cuối (`Shop.Example.` → `shop.example`).
fn host_of(url: &url::Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    host.strip_suffix('.').map(str::to_owned).unwrap_or(host)
}

/// The authority part (`user@host:port`) of an address written as `scheme://authority/…`.
fn authority_of(raw: &str) -> &str {
    let rest = raw.split_once("://").map_or(raw, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or_default()
}

fn invalid_template() -> FieldError {
    FieldError::new(
        "value",
        "kernel.link_template_invalid",
        "must be an `https://` address of at most 2000 characters on one of this tenant's hosts, \
         such as `https://shop.example/reset-password?token={token}`; no user name, spaces or \
         placeholders in the host part",
    )
}

/// Shape check of a link template (the checker of every [`TenantSetting::link`]); the host is checked
/// separately because it needs the tenant's domains.
pub(crate) fn link_template(raw: &str) -> Result<(), FieldError> {
    let raw = raw.trim();
    let clean = raw.len() <= LINK_MAX
        && raw.starts_with("https://")
        && !raw.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        && !authority_of(raw).contains(['{', '}', '@']);
    let parsed = parse(raw).filter(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.domain().is_some()
    });
    if clean && parsed.is_some() { Ok(()) } else { Err(invalid_template()) }
}

/// The placeholders of `setting` missing from `raw`.
fn missing_placeholders(setting: &TenantSetting, raw: &str) -> Vec<&'static str> {
    setting.placeholders.iter().copied().filter(|p| !raw.contains(p)).collect()
}

/// Full check of a link value for tenant `tenant`: shape, placeholders, host. `Ok(Err(problem))` when the value
/// is refused.
pub(crate) async fn check_link(
    trust: &LinkTrust,
    settings: &dyn TenantSettings,
    tenant: &TenantContext,
    setting: &TenantSetting,
    raw: &str,
    server_host: Option<&str>,
) -> Result<Result<(), FieldError>, Error> {
    let raw = raw.trim();
    if let Err(problem) = link_template(raw) {
        return Ok(Err(problem));
    }
    let missing = missing_placeholders(setting, raw);
    if !missing.is_empty() {
        return Ok(Err(FieldError::new(
            "value",
            "kernel.link_placeholder_missing",
            "must contain {placeholders}, where the server puts the value of the link",
        )
        .with_param("placeholders", missing.join(", "))));
    }
    let host = parse(raw).map(|url| host_of(&url)).unwrap_or_default();
    if !trust.trusts(settings, tenant, &host, server_host).await? {
        return Ok(Err(FieldError::new(
            "value",
            "kernel.link_host_not_allowed",
            "the host `{host}` is not one of this tenant's verified domains nor the host of the \
             setting `kernel.public_url`: verify the domain first (`POST /api/v1/kernel/domains`)",
        )
        .with_param("host", host)));
    }
    Ok(Ok(()))
}

/// Per-tenant links in mail. Cheap to clone. Modules get the kernel's instance with
/// `context.require::<SiteLinks>()`; [`SiteLinks::new`] (no domain check) and the default (no per-tenant
/// configuration, for hand-built tests) remain.
#[derive(Clone, Default)]
pub struct SiteLinks {
    settings: Option<Arc<dyn TenantSettings>>,
    trust: LinkTrust,
}

impl SiteLinks {
    /// Links that follow `kernel.public_url` and the link settings; a link setting is trusted only on the
    /// host of `kernel.public_url` or of the server's link (no verified domains).
    pub fn new(settings: Arc<dyn TenantSettings>) -> Self {
        SiteLinks { settings: Some(settings), trust: LinkTrust::default() }
    }

    pub(crate) fn with_trust(mut self, trust: LinkTrust) -> Self {
        self.trust = trust;
        self
    }

    /// `template` (the server's configured address, may contain `{token}`) for tenant `tenant`: its head
    /// replaced by the tenant's `kernel.public_url`, when set.
    pub async fn rewrite(&self, tenant: &TenantContext, template: &str) -> Result<String, Error> {
        let Some(settings) = &self.settings else { return Ok(template.to_owned()) };
        // Giá trị cũ đặt trước khi có bộ kiểm mà sai dạng: dùng địa chỉ của máy chủ.
        let site = parsed_setting(settings.as_ref(), tenant, &PUBLIC_URL, |raw| {
            site_origin(raw).map(|()| raw.trim().to_owned())
        })
        .await?;
        Ok(site.map_or_else(|| template.to_owned(), |site| rebase(template, &site)))
    }

    /// The link template for `setting` (a [`TenantSetting::link`]) at tenant `tenant`, placeholders not yet
    /// filled: the tenant's own template when set and still valid, otherwise `server` (the server's
    /// configured address) through [`rewrite`](Self::rewrite); `None` when neither exists (the feature that
    /// needs the link is off for this tenant).
    pub async fn link(
        &self,
        tenant: &TenantContext,
        setting: &TenantSetting,
        server: Option<&str>,
    ) -> Result<Option<String>, Error> {
        if let Some(settings) = &self.settings
            && let Some(value) = settings.get(tenant, setting).await?
        {
            let server_host = server.and_then(parse).map(|url| host_of(&url));
            let raw = value.as_str().trim();
            match check_link(
                &self.trust,
                settings.as_ref(),
                tenant,
                setting,
                raw,
                server_host.as_deref(),
            )
            .await?
            {
                Ok(()) => return Ok(Some(raw.to_owned())),
                Err(problem) => tracing::warn!(
                    setting = setting.name(),
                    tenant_id = %tenant.id(),
                    problem = %problem.code,
                    "a tenant link is not accepted any more; the server's link is used"
                ),
            }
        }
        match server {
            Some(template) => Ok(Some(self.rewrite(tenant, template).await?)),
            None => Ok(None),
        }
    }
}

/// Replaces the head of `template` with `site`; a `template` that is not an http(s) address stays unchanged.
pub(super) fn rebase(template: &str, site: &str) -> String {
    let site = site.trim().trim_end_matches('/');
    match (super::origin_of(template), super::origin_of(site)) {
        (Some(origin), Some(_)) => format!("{site}{}", &template[origin.len()..]),
        _ => template.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_phan_dau_giu_duong_dan() {
        assert_eq!(
            rebase("https://www.example.com/confirm?token={token}", "https://shop.acme.vn/"),
            "https://shop.acme.vn/confirm?token={token}"
        );
        assert_eq!(
            rebase("https://api.example.com", "http://localhost:3400"),
            "http://localhost:3400"
        );
        assert_eq!(rebase("not a url {token}", "https://shop.acme.vn"), "not a url {token}");
        assert!(site_origin("https://shop.acme.vn").is_ok());
        assert!(site_origin("https://shop.acme.vn/").is_ok());
        assert!(site_origin("https://shop.acme.vn/vi").is_err());
        assert!(site_origin("ftp://shop").is_err());
    }

    #[test]
    fn mau_link_chi_nhan_https_sach() {
        assert!(link_template("https://shop.example/dat-lai-mat-khau?t={token}").is_ok());
        assert!(link_template("https://Shop.Example/{token}").is_ok());
        for bad in [
            "http://shop.example/r?t={token}",
            "https://user@shop.example/r?t={token}",
            "https://shop.example@evil.example/r?t={token}",
            "https://{token}.evil.example/",
            "https://shop.example{token}",
            "https://evil.example\\@shop.example/{token}",
            "https://shop.example/ r?t={token}",
            "https://10.0.0.1/r?t={token}",
            "javascript:alert(1)//{token}",
            "//shop.example/{token}",
        ] {
            assert!(link_template(bad).is_err(), "{bad}");
        }
        assert!(link_template(&format!("https://shop.example/{}", "a".repeat(2000))).is_err());
    }

    #[test]
    fn host_doc_nhu_trinh_duyet() {
        let host = |raw: &str| parse(raw).map(|url| host_of(&url));
        assert_eq!(host("https://Shop.Example./a?t={token}").as_deref(), Some("shop.example"));
        assert_eq!(host("https://shop.example:8443/{token}").as_deref(), Some("shop.example"));
    }
}
