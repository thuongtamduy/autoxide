//! Chặn webhook gọi vào mạng nội bộ (SSRF, D-149): người tạo webhook chọn địa chỉ, nên
//! không có lọc thì máy chủ thành công cụ dò và gọi dịch vụ chỉ mở trong mạng riêng (vd
//! `169.254.169.254` của nhà cung cấp cloud, database, trang quản trị nội bộ).
//!
//! Lọc ở hai chỗ:
//! - lúc tạo hay sửa webhook: địa chỉ là IP (hay `localhost`) không public thì từ chối ngay;
//! - lúc gửi: tên miền được phân giải bằng [`PublicOnly`], bỏ mọi IP không public. Lọc lúc
//!   kết nối (không phải lúc kiểm URL) chặn được cả "DNS rebinding" — tên miền trả IP public
//!   khi kiểm rồi IP nội bộ khi gửi.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// Câu giải thích khi địa chỉ nằm trong mạng riêng.
pub(crate) const PRIVATE_ADDRESS: &str = "points to a private or local network address; \
     webhooks are sent only to public addresses unless the server sets \
     `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS=true`";

/// `ip` có phải địa chỉ public trên Internet không (không loopback, mạng riêng, link-local,
/// CGNAT, multicast, địa chỉ dành cho tài liệu hay dự trữ).
pub(crate) fn is_public(ip: IpAddr) -> bool {
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
pub(crate) fn is_local_name(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost" || host.ends_with(".localhost")
}

/// Host của `url` không bị biết trước là nội bộ: IP thì phải public, tên thì không phải
/// `localhost` (tên miền khác được kiểm lúc gửi, bằng [`PublicOnly`]).
pub(crate) fn is_public_host(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    // IPv6 trong URL nằm trong ngoặc vuông: `https://[::1]/`.
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    match bare.parse::<IpAddr>() {
        Ok(ip) => is_public(ip),
        Err(_) => !is_local_name(host),
    }
}

/// Bộ phân giải tên miền chỉ trả IP public; không còn IP nào thì báo lỗi.
pub(crate) struct PublicOnly;

/// Lỗi khi tên miền chỉ trỏ vào mạng riêng — nhận ra được trong chuỗi lỗi của reqwest.
#[derive(Debug)]
pub(crate) struct PrivateAddress;

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
            if is_local_name(&host) {
                return Err(PrivateAddress.into());
            }
            // Cổng 0: reqwest thay bằng cổng của URL.
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|address| is_public(address.ip()))
                .collect();
            if found.is_empty() {
                return Err(PrivateAddress.into());
            }
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

    #[tokio::test]
    async fn ten_mien_chi_tro_vao_may_nay_bi_tu_choi() {
        for host in ["localhost", "127.0.0.1"] {
            let error = PublicOnly.resolve(host.parse().unwrap()).await.err().unwrap();
            assert!(error.is::<PrivateAddress>(), "{host}: {error}");
        }
    }
}
