//! Addresses a person's browser is sent back to after a step at an outside provider (`return_url` of a payment,
//! of an identity check): only addresses that belong to the tenant, so a real link of a shop cannot land the
//! person on a stranger's page (open redirect; P-5 and L-11 of the 2026-11 security review,
//! D-20261125-return-url-allowlist, D-20261128-return-url-kernel).
//!
//! An address is allowed when its origin (scheme + host + port, read like a browser reads it — `user@host`,
//! `\` or upper case cannot trick the comparison) is `https://` on one of the tenant's verified domains
//! ([`TenantDomains::verified_hosts`]), or one of the origins the tenant listed in a setting of the calling
//! module.

use super::{TenantContext, TenantDomains};
use crate::error::Error;

/// Maximum length of a return address.
pub const RETURN_URL_MAX_LEN: usize = 2000;

/// Why a return address was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnUrlProblem {
    /// Not an http(s) URL of at most [`RETURN_URL_MAX_LEN`] characters (or it carries a user name, spaces or
    /// control characters).
    Format,
    /// Well formed, but not on one of the tenant's own addresses.
    NotAllowed,
}

// URL http(s) đã đọc như trình duyệt, không có user/mật khẩu, không có khoảng trắng hay ký tự điều khiển ở hai
// đầu (trình duyệt bỏ chúng đi, nên chuỗi chứa chúng là chuỗi khác với chuỗi được kiểm); `None` nếu không phải.
fn parse(raw: &str) -> Option<url::Url> {
    if raw.is_empty() || raw.chars().any(char::is_control) || raw.trim() != raw {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    let plain = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some();
    plain.then_some(url)
}

// Origin chuẩn hoá `scheme://host:port` của `url` (tên miền chữ thường, IDN thành punycode, bỏ dấu chấm cuối).
fn origin_of(url: &url::Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_suffix('.').unwrap_or(&host).to_owned();
    let port = url.port_or_known_default()?;
    Some(format!("{}://{host}:{port}", url.scheme()))
}

// Một origin hợp lệ để khai: chỉ scheme + host (+ port), không đường dẫn, tham số hay ký tự đại diện `*` (so khớp
// chính xác; `https://*.example.com` sẽ không bao giờ khớp nên từ chối ngay lúc lưu để quản trị viên khỏi hiểu lầm).
fn listed_origin(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.contains('*') {
        return None;
    }
    let url = parse(raw)?;
    let bare = url.path() == "/" && url.query().is_none() && url.fragment().is_none();
    bare.then(|| origin_of(&url)).flatten()
}

/// The items of a comma-separated list of origins (a tenant setting such as
/// `https://shop.example.com, http://localhost:3000`) that are not bare origins: with a path, a query, a `*`, or
/// not http(s). Empty when the list is valid — use it to check the setting when it is saved.
pub fn invalid_return_origins(raw: &str) -> Vec<&str> {
    raw.split(',').map(str::trim).filter(|item| listed_origin(item).is_none()).collect()
}

/// Checks return address `url` for tenant `tenant`: `Ok(Ok(normalized))` — the string to store and hand to the
/// provider, in the browser's normal form — when it is `https://` on a verified domain of the tenant
/// (`domains`, multi-tenant mode) or matches an origin of `listed` (the tenant's comma-separated setting);
/// otherwise `Ok(Err(problem))`.
pub async fn check_return_url(
    tenant: &TenantContext,
    url: &str,
    domains: Option<&TenantDomains>,
    listed: Option<&str>,
) -> Result<Result<String, ReturnUrlProblem>, Error> {
    let parsed = parse(url).filter(|_| url.len() <= RETURN_URL_MAX_LEN);
    let Some((parsed, wanted)) = parsed.and_then(|parsed| origin_of(&parsed).map(|o| (parsed, o)))
    else {
        return Ok(Err(ReturnUrlProblem::Format));
    };
    let normalized = parsed.as_str().to_owned();
    if let Some(domains) = domains {
        let hosts = domains.verified_hosts(tenant).await?;
        if hosts.iter().any(|host| wanted == format!("https://{host}:443")) {
            return Ok(Ok(normalized));
        }
    }
    if let Some(listed) = listed
        && listed.split(',').filter_map(listed_origin).any(|origin| origin == wanted)
    {
        return Ok(Ok(normalized));
    }
    Ok(Err(ReturnUrlProblem::NotAllowed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(raw: &str) -> Option<String> {
        parse(raw).as_ref().and_then(origin_of)
    }

    #[test]
    fn origin_doc_nhu_trinh_duyet() {
        assert_eq!(
            origin("https://Shop.Example/x?y=1").as_deref(),
            Some("https://shop.example:443")
        );
        assert_eq!(origin("http://localhost:3000/done").as_deref(), Some("http://localhost:3000"));
        // `user@host`: trình duyệt tới `evil.example`, không phải `shop.example`.
        assert_eq!(origin("https://shop.example@evil.example/"), None);
        assert_eq!(
            origin("https://evil.example\\@shop.example/").as_deref(),
            Some("https://evil.example:443")
        );
        assert_eq!(origin(" https://shop.example/"), None);
        assert_eq!(origin("javascript:alert(1)"), None);
        assert_eq!(origin("myapp://done"), None);
    }

    #[test]
    fn danh_sach_origin_khai() {
        assert!(invalid_return_origins("https://shop.example, http://localhost:3000").is_empty());
        assert_eq!(
            invalid_return_origins("https://shop.example/path, https://*.example.com, ftp://x"),
            vec!["https://shop.example/path", "https://*.example.com", "ftp://x"]
        );
    }

    #[tokio::test]
    async fn chi_nhan_origin_da_khai() {
        let tenant = crate::tenancy::test_tenant();
        let listed = Some("https://shop.example, http://localhost:3000");
        let check = async |url: &str| check_return_url(&tenant, url, None, listed).await.unwrap();
        assert_eq!(
            check("https://shop.example/kyc?x=1").await,
            Ok("https://shop.example/kyc?x=1".to_owned())
        );
        assert_eq!(check("http://localhost:3000/").await, Ok("http://localhost:3000/".to_owned()));
        assert_eq!(
            check("https://shop.example.evil.example/").await,
            Err(ReturnUrlProblem::NotAllowed)
        );
        assert_eq!(check("http://shop.example/").await, Err(ReturnUrlProblem::NotAllowed));
        assert_eq!(check("not a url").await, Err(ReturnUrlProblem::Format));
        let none = check_return_url(&tenant, "https://shop.example/", None, None).await.unwrap();
        assert_eq!(none, Err(ReturnUrlProblem::NotAllowed));
    }
}
