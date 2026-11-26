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
//! A link setting and `kernel.public_url` are untrusted input (any tenant administrator sets them)
//! (D-20261125-tenant-site-verified):
//!
//! - **Multi-tenant mode:** the host must be a domain this tenant has `verified` right now, or the platform host
//!   (`AUTOXIDE_HTTP_PUBLIC_URL`); `https` only, default port only. Pending, failed, revoked, never added, or
//!   another tenant's domain: refused when set (422), ignored with a warning when used (the server's link is
//!   used). A value stored before this rule is kept but ignored, and `GET /api/v1/kernel/settings` flags it.
//! - **Single-tenant mode** (no domain table to check against): a link setting must be on the platform host
//!   or on the address of `kernel.public_url`; `kernel.public_url` must be `https` unless the host is
//!   `localhost` / `127.0.0.1` (development).
//!
//! A link setting also needs `https`, no user name, no placeholder in the host part, every placeholder the
//! module fills. Domain owners come from the registry's snapshot of verified domains (shared with CORS; no
//! query per mail); a domain beyond the snapshot's cap is not in it, so it is not trusted (fail closed).

use std::sync::Arc;

use super::{PUBLIC_URL, TenantSetting, TenantSettings, parsed_setting, site_origin};
use crate::error::{Error, FieldError};
use crate::tenancy::{DatabaseRegistry, TenancyMode, TenantContext, TenantId};

/// Group of the per-tenant link settings ([`TenantSetting::link`]).
pub const LINK_GROUP: &str = "link";

/// Longest accepted link template.
const LINK_MAX: usize = 2000;

/// What a tenant's link host is checked against: owners of verified domains and the platform host.
#[derive(Clone, Default)]
pub(crate) struct LinkTrust {
    registry: Option<DatabaseRegistry>,
    platform_host: Option<String>,
}

/// Code of every refusal of a `kernel.public_url` value.
const PUBLIC_URL_CODE: &str = "kernel.public_url_not_allowed";

impl LinkTrust {
    pub(crate) fn new(registry: Option<DatabaseRegistry>, platform_host: Option<String>) -> Self {
        LinkTrust { registry, platform_host }
    }

    /// The installation runs in multi-tenant mode (the strict rule applies).
    fn multi(&self) -> bool {
        self.registry.as_ref().is_some_and(|registry| registry.mode() == TenancyMode::Multi)
    }

    /// The tenant that has verified domain `host`, if any: the in-memory snapshot of every verified domain that
    /// CORS uses too (one source of truth; a host nobody verified costs no query).
    async fn owner(&self, host: &str) -> Result<Option<TenantId>, Error> {
        match &self.registry {
            Some(registry) => registry.verified_domain_owner(host).await,
            None => Ok(None),
        }
    }

    /// Why `url` may not be an address of tenant `tenant` in multi-tenant mode (see the module documentation);
    /// `None` when it may, and always `None` in single-tenant mode.
    async fn multi_problem(
        &self,
        tenant: &TenantContext,
        url: &url::Url,
    ) -> Result<Option<FieldError>, Error> {
        if !self.multi() {
            return Ok(None);
        }
        let host = host_of(url);
        if url.scheme() != "https" {
            return Ok(Some(
                FieldError::new("value", PUBLIC_URL_CODE, "must start with `https://`")
                    .with_param("host", host),
            ));
        }
        let owner = self.owner(&host).await?;
        let own = owner == Some(tenant.id());
        let platform = owner.is_none() && self.platform_host.as_deref() == Some(host.as_str());
        if let Some(owner) = owner
            && owner != tenant.id()
        {
            return Ok(Some(
                FieldError::new(
                    "value",
                    PUBLIC_URL_CODE,
                    "`{host}` is a verified domain of another tenant",
                )
                .with_param("host", host),
            ));
        }
        if !own && !platform {
            return Ok(Some(
                FieldError::new(
                    "value",
                    PUBLIC_URL_CODE,
                    "`{host}` is not a verified domain of this tenant: add it with `POST \
                     /api/v1/kernel/domains` and verify it first",
                )
                .with_param("host", host),
            ));
        }
        if url.port().is_some() {
            return Ok(Some(
                FieldError::new(
                    "value",
                    PUBLIC_URL_CODE,
                    "use `{host}` without a port: a verified domain is trusted on its default port only",
                )
                .with_param("host", host),
            ));
        }
        Ok(None)
    }

    /// The tenant's `kernel.public_url`, unless it is not allowed now (then a warning, and `None`: links keep
    /// the server's address).
    pub(crate) async fn public_site(
        &self,
        settings: &dyn TenantSettings,
        tenant: &TenantContext,
    ) -> Result<Option<String>, Error> {
        let site = parsed_setting(settings, tenant, &PUBLIC_URL, |raw| {
            site_origin(raw).map(|()| raw.trim().to_owned())
        })
        .await?;
        let Some(site) = site else { return Ok(None) };
        let Some(url) = parse(&site) else { return Ok(None) };
        match self.multi_problem(tenant, &url).await? {
            Some(problem) => {
                tracing::warn!(
                    tenant_id = %tenant.id(),
                    problem = %problem.template(),
                    "kernel.public_url is not allowed; the server's links are used"
                );
                Ok(None)
            }
            None => Ok(Some(site)),
        }
    }

    /// Whether the address `url` belongs to tenant `tenant` (see the module documentation).
    pub(crate) async fn trusts(
        &self,
        settings: &dyn TenantSettings,
        tenant: &TenantContext,
        url: &url::Url,
    ) -> Result<bool, Error> {
        if self.multi() {
            return Ok(self.multi_problem(tenant, url).await?.is_none());
        }
        let host = host_of(url);
        if self.platform_host.as_deref() == Some(host.as_str()) && url.port().is_none() {
            return Ok(true);
        }
        let site = self.public_site(settings, tenant).await?;
        Ok(site.as_deref().and_then(parse).is_some_and(|site| {
            host_of(&site) == host && site.port_or_known_default() == url.port_or_known_default()
        }))
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
         with the path and placeholders of your page; no user name, spaces or placeholders in the \
         host part",
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

/// Check of a new `kernel.public_url` value for tenant `tenant`: the multi-tenant rule, or in single-tenant mode
/// `https` unless the host is `localhost` / `127.0.0.1`.
pub(crate) async fn check_public_url(
    trust: &LinkTrust,
    tenant: &TenantContext,
    raw: &str,
) -> Result<Result<(), FieldError>, Error> {
    let Some(url) = parse(raw.trim()) else {
        // Trình duyệt không đọc được (vd port 99999): lúc dùng sẽ bị bỏ qua, nên từ chối ngay lúc đặt.
        return Ok(Err(FieldError::new(
            "value",
            "invalid",
            "must be an address such as `https://shop.example`, with no path",
        )));
    };
    if !trust.multi() {
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1"));
        if url.scheme() != "https" && !local {
            return Ok(Err(FieldError::new(
                "value",
                PUBLIC_URL_CODE,
                "must start with `https://` (`http://` only for `localhost` or `127.0.0.1`)",
            )));
        }
        return Ok(Ok(()));
    }
    Ok(trust.multi_problem(tenant, &url).await?.map_or(Ok(()), Err))
}

/// The code to show next to a stored value of `setting` that is not used any more (`GET /kernel/settings`):
/// `kernel.public_url_unverified` or `kernel.link_unverified`; `None` when the value is used.
pub(crate) async fn stored_problem(
    trust: &LinkTrust,
    settings: &dyn TenantSettings,
    tenant: &TenantContext,
    setting: &TenantSetting,
    raw: &str,
) -> Result<Option<&'static str>, Error> {
    if setting.name() == PUBLIC_URL.name() {
        let refused = match parse(raw.trim()) {
            Some(url) => trust.multi_problem(tenant, &url).await?.is_some(),
            // Không đọc được (vd port 99999): lúc dùng bị bỏ qua, nên cũng báo.
            None => true,
        };
        return Ok(refused.then_some("kernel.public_url_unverified"));
    }
    if setting.group_name() == Some(LINK_GROUP) {
        let refused = check_link(trust, settings, tenant, setting, raw).await?.is_err();
        return Ok(refused.then_some("kernel.link_unverified"));
    }
    Ok(None)
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
    let Some(url) = parse(raw) else { return Ok(Err(invalid_template())) };
    let host = host_of(&url);
    if !trust.trusts(settings, tenant, &url).await? {
        return Ok(Err(FieldError::new(
            "value",
            "kernel.link_host_not_allowed",
            "the host `{host}` (with its port) is not allowed: use a verified domain of this tenant \
             on its default port (verify it first with `POST /api/v1/kernel/domains`), or the \
             platform's own host",
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
    /// Links that follow `kernel.public_url` and the link settings, without the domain registry: a link setting
    /// is trusted only on the address of `kernel.public_url`.
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
        // Giá trị sai dạng, hay host là tên miền đã xác minh của khách KHÁC: dùng địa chỉ của máy chủ.
        let site = self.trust.public_site(settings.as_ref(), tenant).await?;
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
            let raw = value.as_str().trim();
            match check_link(&self.trust, settings.as_ref(), tenant, setting, raw).await? {
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
        assert!(site_origin("https://{token}.shop.example").is_err());
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

    // Bản dịch tiếng Việt của mọi lỗi ô ở đây điền được (không còn `{…}` thiếu giá trị).
    #[test]
    fn loi_o_co_ban_dich_tieng_viet() {
        let translations = crate::i18n::Translations::new(Vec::new()).unwrap();
        for problem in [invalid_template(), link_template("http://x.example/{token}").unwrap_err()]
        {
            let vi = translations.field_message(&problem, crate::i18n::Language::Vi);
            assert_ne!(vi, problem.message, "{}", problem.message);
        }
    }

    #[test]
    fn host_doc_nhu_trinh_duyet() {
        let host = |raw: &str| parse(raw).map(|url| host_of(&url));
        assert_eq!(host("https://Shop.Example./a?t={token}").as_deref(), Some("shop.example"));
        assert_eq!(host("https://shop.example:8443/{token}").as_deref(), Some("shop.example"));
    }
}
