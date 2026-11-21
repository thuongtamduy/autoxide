//! Băm địa chỉ IP để đếm và lưu mà không giữ IP thô (D-115).
//!
//! 1. Gộp theo tiền tố: IPv4 theo khối /24, IPv6 theo khối /64 (một nhà, một máy thường giữ cả
//!    khối và đổi địa chỉ trong khối) — cũng là cách rate limiter gộp IPv6.
//! 2. HMAC-SHA256 bằng một khoá dẫn từ khoá ký của kernel ([`SecretKey`]), giữ 16 byte đầu,
//!    mã hoá base64url (22 ký tự). Không có khoá thì không dò ngược được IP từ bản băm (không
//!    gian IPv4 nhỏ, băm thường thì thử hết được).
//!
//! Hai dạng:
//!
//! - [`bucket`]: khoá đổi theo ngày (UTC) — cho bộ đếm (vd "bao nhiêu lần gửi từ khối này hôm
//!   nay"). Bản băm của hai ngày không nối được với nhau.
//! - [`fingerprint`]: khoá cố định — lưu cùng bản ghi (bình luận, bài gửi form) để cấm hay gom
//!   theo người gửi; module tự xoá sau thời hạn của mình.
//!
//! Cả hai nhận `scope` (tên module, hay phạm vi form): mỗi phạm vi một khoá riêng (D-153), nên
//! dấu của cùng một người ở `forms` và ở `comments` khác nhau — đọc được database của module
//! này không nối được người đó sang module kia. Hai bản ghi chỉ so được với nhau khi cùng
//! phạm vi.
//!
//! Giới hạn: khoá gốc nằm trong database (không đặt `AUTOXIDE_SECRET_KEY`) thì ai có bản sao
//! database cũng tính lại được khoá và thử hết các khối IPv4 (2^24). Máy chủ cảnh báo lúc khởi
//! động; khi chạy thật hãy đặt khoá ngoài database.

use crate::time::Timestamp;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::error::Error;
use crate::secret::SecretKey;

/// Khối địa chỉ chứa `ip`: /24 cho IPv4, /64 cho IPv6. Địa chỉ IPv4 viết dạng IPv6
/// (`::ffff:a.b.c.d`) tính như IPv4.
pub fn prefix(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4_block(v4)),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4_block(v4)),
            None => {
                let s = v6.segments();
                IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
    }
}

fn v4_block(v4: Ipv4Addr) -> Ipv4Addr {
    let [a, b, c, _] = v4.octets();
    Ipv4Addr::new(a, b, c, 0)
}

/// Bản băm của khối chứa `ip` trong phạm vi `scope`, khoá đổi theo ngày UTC của `now`. Dùng
/// cho bộ đếm.
pub async fn bucket(
    secret: &SecretKey,
    scope: &str,
    ip: IpAddr,
    now: Timestamp,
) -> Result<String, Error> {
    let key = secret
        .derive(&format!("autoxide/ip-hash/bucket/v2/{scope}/{}", now.format("%Y-%m-%d")))
        .await?;
    digest(&key, ip)
}

/// Bản băm cố định của khối chứa `ip` trong phạm vi `scope` (tên module). Lưu cùng bản ghi;
/// module xoá sau thời hạn của mình.
pub async fn fingerprint(secret: &SecretKey, scope: &str, ip: IpAddr) -> Result<String, Error> {
    let key = secret.derive(&format!("autoxide/ip-hash/fingerprint/v2/{scope}")).await?;
    digest(&key, ip)
}

fn digest(key: &[u8], ip: IpAddr) -> Result<String, Error> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
    mac.update(prefix(ip).to_string().as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(URL_SAFE_NO_PAD.encode(&bytes[..16]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn gop_theo_khoi() {
        assert_eq!(prefix(ip("203.0.113.77")), ip("203.0.113.0"));
        assert_eq!(prefix(ip("2001:db8:1:2:aaaa:bbbb:cccc:dddd")), ip("2001:db8:1:2::"));
        assert_eq!(prefix(ip("::ffff:203.0.113.77")), ip("203.0.113.0"));
    }

    #[test]
    fn cung_khoi_cung_bam_khac_khoi_khac_bam() {
        let key = [7u8; 32];
        assert_eq!(
            digest(&key, ip("203.0.113.1")).unwrap(),
            digest(&key, ip("203.0.113.254")).unwrap()
        );
        assert_ne!(
            digest(&key, ip("203.0.113.1")).unwrap(),
            digest(&key, ip("203.0.114.1")).unwrap()
        );
        assert_eq!(
            digest(&key, ip("2001:db8::1")).unwrap(),
            digest(&key, ip("2001:db8::ffff")).unwrap()
        );
        assert_eq!(digest(&key, ip("203.0.113.1")).unwrap().len(), 22);
        // Khoá khác: bản băm khác.
        assert_ne!(
            digest(&key, ip("203.0.113.1")).unwrap(),
            digest(&[8u8; 32], ip("203.0.113.1")).unwrap()
        );
    }
}
