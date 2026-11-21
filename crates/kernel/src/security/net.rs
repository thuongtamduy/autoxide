//! Chặn gọi ra ngoài vào mạng nội bộ (SSRF, D-149): khi người dùng hay quản trị khách chọn địa
//! chỉ máy chủ gọi tới (webhook, issuer OIDC), không có lọc thì máy chủ thành công cụ dò và gọi
//! dịch vụ chỉ mở trong mạng riêng (vd `169.254.169.254` của nhà cung cấp cloud, database, trang
//! quản trị nội bộ). Dời từ `webhooks` (bước 40) để mọi module dùng một cách làm.
//!
//! Lọc ở hai chỗ:
//! - lúc nhận địa chỉ: IP (hay `localhost`) không public thì từ chối ngay ([`is_public_host`]) —
//!   reqwest KHÔNG gọi bộ phân giải cho URL là IP, nên kiểm này bắt buộc trước mỗi lần gọi;
//! - lúc gọi: tên miền được phân giải bằng [`PublicOnly`], bỏ mọi IP không public. Lọc lúc
//!   kết nối (không phải lúc kiểm URL) chặn được cả "DNS rebinding" — tên miền trả IP public
//!   khi kiểm rồi IP nội bộ khi gọi.
//!
//! Giao thức không đi qua reqwest (SMTP của khách…) dùng cùng hai bước: [`check_target_url`] lúc
//! lưu, [`resolve_target`] ngay trước khi kết nối rồi kết nối vào ĐÚNG IP nó trả (D-20261121-ssrf-targets).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::error::FieldError;

/// URL đã kiểm (dùng lại kiểu của reqwest để module không phải khai thêm thư viện `url`).
pub use reqwest::Url;

/// `ip` có phải địa chỉ public trên Internet không (không loopback, mạng riêng, link-local,
/// CGNAT, multicast, địa chỉ dành cho tài liệu hay dự trữ).
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

/// Tên máy chỉ máy này (`localhost`, `*.localhost`), không cần phân giải cũng biết.
pub fn is_local_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

/// Host của `url` không bị biết trước là nội bộ: IP thì phải public, tên thì không phải
/// `localhost` (tên miền khác được kiểm lúc gửi, bằng [`PublicOnly`]).
pub fn is_public_host(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(is_public_host_name)
}

/// Như [`is_public_host`], cho host trần (`smtp.example.com`, `10.0.0.1`, `[::1]`).
pub fn is_public_host_name(host: &str) -> bool {
    // IPv6 trong URL nằm trong ngoặc vuông: `https://[::1]/`.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(ip) => is_public(ip),
        Err(_) => !bare.is_empty() && !is_local_name(bare),
    }
}

/// Kiểm địa chỉ người dùng chọn cho máy chủ gọi tới, lúc LƯU (cấu hình của khách): đọc được là URL,
/// scheme thuộc `schemes`, có host, và — trừ khi `allow_private` — host không bị biết trước là nội
/// bộ (IP không public, `localhost`). Tên miền khác chỉ biết lúc kết nối: [`resolve_target`]. Lỗi
/// là lỗi của ô `value` (như bộ kiểm cấu hình của khách).
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

/// Phân giải `host` (tên hay IP) ngay trước khi kết nối và chỉ giữ IP public (trừ khi
/// `allow_private`); không còn IP nào thì lỗi [`PrivateAddress`]. Bên gọi kết nối vào chính các
/// địa chỉ trả về — không phân giải lại — nên tên miền không đổi được sang IP nội bộ giữa lúc kiểm và
/// lúc gọi (DNS rebinding).
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

/// Bộ phân giải tên miền chỉ trả IP public; không còn IP nào thì báo lỗi.
pub struct PublicOnly;

/// Lỗi khi tên miền chỉ trỏ vào mạng riêng — nhận ra được trong chuỗi lỗi của reqwest.
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
