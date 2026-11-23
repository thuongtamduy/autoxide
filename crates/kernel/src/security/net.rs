//! Blocks outbound calls into internal networks (SSRF, D-149): when a user or tenant admin chooses an address
//! the server calls (webhook, OIDC issuer), without filtering the server becomes a probing tool and calls
//! services that are only open on the private network (e.g. the cloud provider's `169.254.169.254`, the
//! database, internal admin pages). Moved from `webhooks` (step 40) so every module uses one approach.
//!
//! Filtering happens in two places:
//! - when the address is received: an IP (or `localhost`) that is not public is rejected immediately
//!   ([`is_public_host`]) — reqwest does NOT call the resolver for a URL that is an IP, so this check is
//!   mandatory before every call;
//! - when calling: a domain name is resolved with [`PublicOnly`], dropping every non-public IP. Filtering at
//!   connect time (not at URL check time) also blocks "DNS rebinding" — a domain that returns a public IP
//!   at check time and then an internal IP at call time.
//!
//! Protocols that do not go through reqwest (a tenant's SMTP…) use the same two steps: [`check_target_url`]
//! at save time, [`resolve_target`] right before connecting, then connect to EXACTLY the IP it returns
//! (D-20261121-ssrf-targets).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::error::FieldError;

/// A validated URL (reusing reqwest's type so modules need not declare the `url` library as well).
pub use reqwest::Url;

/// Whether `ip` is a public Internet address (not loopback, private network, link-local,
/// CGNAT, multicast, documentation-reserved or reserved).
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0 // "mạng này"
        || (a == 100 && (64..128).contains(&b)) // CGNAT 100.64.0.0/10
        || (a == 192 && b == 0 && c == 0) // 192.0.0.0/24, dành cho giao thức
        || (a == 192 && b == 88 && c == 99) // 6to4 relay anycast 192.88.99.0/24
        || (a == 198 && (18..20).contains(&b)) // đo hiệu năng 198.18.0.0/15
        || a >= 240) // dự trữ 240.0.0.0/4
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    // IPv4 nằm trong IPv6 (`::ffff:a.b.c.d`, NAT64 `64:ff9b::a.b.c.d`): xét phần IPv4.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let segments = ip.segments();
    let embedded = |high: u16, low: u16| {
        let [a, b] = high.to_be_bytes();
        let [c, d] = low.to_be_bytes();
        Ipv4Addr::new(a, b, c, d)
    };
    // NAT64 `64:ff9b::/96` và IPv4-compatible `::a.b.c.d` (`::/96`, Linux định tuyến qua đường
    // hầm `sit0`): IPv4 ở 32 bit cuối.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] || segments[..6] == [0, 0, 0, 0, 0, 0] {
        return !ip.is_unspecified()
            && !ip.is_loopback()
            && is_public_v4(embedded(segments[6], segments[7]));
    }
    // 6to4 `2002::/16`: IPv4 ở bit 16–47 (D-157).
    if segments[0] == 0x2002 {
        return is_public_v4(embedded(segments[1], segments[2]));
    }
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (segments[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
        || (segments[0] & 0xffc0) == 0xfec0 // site-local fec0::/10 (bỏ rồi, vẫn có nơi dùng)
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // tài liệu 2001:db8::/32
        || (segments[0] == 0x2001 && segments[1] == 0) // Teredo 2001::/32
        || (segments[0] == 0x64 && segments[1] == 0xff9b && segments[2] == 1)) // NAT64 nội bộ 64:ff9b:1::/48
}

/// Host names that mean this machine only (`localhost`, `*.localhost`), known without resolving.
pub fn is_local_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

/// The host of `url` is not known in advance to be internal: an IP must be public, a name must not be
/// `localhost` (other domain names are checked at send time, with [`PublicOnly`]).
pub fn is_public_host(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(is_public_host_name)
}

/// Like [`is_public_host`], for a bare host (`smtp.example.com`, `10.0.0.1`, `[::1]`).
pub fn is_public_host_name(host: &str) -> bool {
    // IPv6 trong URL nằm trong ngoặc vuông: `https://[::1]/`.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(ip) => is_public(ip),
        Err(_) => !bare.is_empty() && !is_local_name(bare),
    }
}

/// Validates an address a user chose for the server to call, when SAVING (tenant configuration): it must
/// parse as a URL, the scheme must be in `schemes`, it must have a host, and — unless `allow_private` — the
/// host must not be known in advance to be internal (non-public IP, `localhost`). Other domain names are only
/// known at connect time: [`resolve_target`]. The error is an error of the `value` field (like the tenant
/// configuration validator).
pub fn check_target_url(
    raw: &str,
    schemes: &[&str],
    allow_private: bool,
) -> Result<Url, FieldError> {
    let invalid = || {
        FieldError::new(
            "value",
            "invalid",
            "must be an address such as `{example}`, with one of the schemes {schemes}",
        )
        .with_param("example", format!("{}://host.example", schemes.first().unwrap_or(&"https")))
        .with_param(
            "schemes",
            schemes.iter().map(|s| format!("`{s}`")).collect::<Vec<_>>().join(", "),
        )
    };
    let url = Url::parse(raw.trim()).map_err(|_| invalid())?;
    if !schemes.contains(&url.scheme()) {
        return Err(invalid());
    }
    let Some(host) = url.host_str().filter(|host| !host.is_empty()) else {
        return Err(invalid());
    };
    if !allow_private && !is_public_host_name(host) {
        return Err(FieldError::new(
            "value",
            "private_address",
            "must point to a server on the public Internet, not a private or local network",
        ));
    }
    Ok(url)
}

/// Resolves `host` (name or IP) right before connecting and keeps only public IPs (unless `allow_private`);
/// if no IP remains the error is [`PrivateAddress`]. The caller connects to exactly the returned addresses —
/// without resolving again — so a domain name cannot switch to an internal IP between the check and the call
/// (DNS rebinding).
pub async fn resolve_target(
    host: &str,
    port: u16,
    allow_private: bool,
) -> Result<Vec<SocketAddr>, std::io::Error> {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if !allow_private && is_local_name(bare) {
        return Err(std::io::Error::other(PrivateAddress));
    }
    let found: Vec<SocketAddr> = tokio::net::lookup_host((bare, port))
        .await?
        .filter(|address| allow_private || is_public(address.ip()))
        .collect();
    if found.is_empty() {
        return Err(std::io::Error::other(PrivateAddress));
    }
    Ok(found)
}

/// Domain name resolver that returns only public IPs; if none remain it reports an error.
pub struct PublicOnly;

/// Error when a domain name points only into the private network — recognizable in reqwest's error chain.
#[derive(Debug)]
pub struct PrivateAddress;

impl std::fmt::Display for PrivateAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the host resolves only to private or local network addresses")
    }
}

impl std::error::Error for PrivateAddress {}

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        Box::pin(async move {
            // Cổng 0: reqwest thay bằng cổng của URL.
            let found = resolve_target(&host, 0, false).await.map_err(|error| {
                // Giữ `PrivateAddress` là lỗi gốc: bên gọi nhận ra nó trong chuỗi lỗi.
                match error.into_inner() {
                    Some(inner) if inner.is::<PrivateAddress>() => {
                        Box::new(PrivateAddress) as Box<dyn std::error::Error + Send + Sync>
                    }
                    Some(inner) => inner,
                    None => Box::new(std::io::Error::other("lookup failed")),
                }
            })?;
            let addresses: Addrs = Box::new(found.into_iter());
            Ok(addresses)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chi_dia_chi_public_moi_duoc_goi() {
        let blocked = [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.2.10",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            // D-157: IPv4 nằm trong IPv6 theo các cách khác, và các dải nội bộ còn lại.
            "::a9fe:a9fe",
            "::7f00:1",
            "2002:a9fe:a9fe::1",
            "2002:7f00:1::",
            "64:ff9b:1::1",
            "fec0::1",
            "2001::1",
            "192.88.99.1",
        ];
        for ip in blocked {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "93.184.216.34",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "::808:808",
            "2002:808:808::1",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        assert!(is_local_name("localhost"));
        assert!(is_local_name("api.LOCALHOST."));
        assert!(!is_local_name("localhost.example.com"));
    }

    #[test]
    fn dia_chi_luu_phai_dung_scheme_va_khong_noi_bo() {
        let smtp = ["smtp", "smtps"];
        assert!(check_target_url("smtps://u:p@smtp.example.com:465", &smtp, false).is_ok());
        assert!(check_target_url("smtp://smtp.example.com?tls=required", &smtp, false).is_ok());
        for raw in ["http://smtp.example.com", "not a url", "smtp://", "file:///etc/passwd"] {
            let problem = check_target_url(raw, &smtp, false).unwrap_err();
            assert_eq!(problem.code, "invalid", "{raw}");
        }
        for raw in [
            "smtp://127.0.0.1:6379",
            "smtp://localhost:25",
            "smtps://[::1]:465",
            "smtp://169.254.169.254",
            "smtp://10.0.0.5:2525",
            "smtp://db.localhost",
        ] {
            let problem = check_target_url(raw, &smtp, false).unwrap_err();
            assert_eq!(problem.code, "private_address", "{raw}");
            assert!(check_target_url(raw, &smtp, true).is_ok(), "{raw}");
        }
    }

    #[tokio::test]
    async fn phan_giai_dich_chi_tra_ip_public() {
        for host in ["localhost", "127.0.0.1", "[::1]", "10.1.2.3"] {
            let error = resolve_target(host, 25, false).await.unwrap_err();
            assert!(error.into_inner().unwrap().is::<PrivateAddress>(), "{host}");
        }
        let found = resolve_target("127.0.0.1", 2525, true).await.unwrap();
        assert_eq!(found, vec!["127.0.0.1:2525".parse().unwrap()]);
        let public = resolve_target("93.184.216.34", 25, false).await.unwrap();
        assert_eq!(public, vec!["93.184.216.34:25".parse().unwrap()]);
    }

    #[tokio::test]
    async fn ten_mien_chi_tro_vao_may_nay_bi_tu_choi() {
        for host in ["localhost", "127.0.0.1"] {
            let error = PublicOnly.resolve(host.parse().unwrap()).await.err().unwrap();
            assert!(error.is::<PrivateAddress>(), "{host}: {error}");
        }
    }
}
