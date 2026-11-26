//! Hashes IP addresses so they can be counted and stored without keeping the raw IP (D-115).
//!
//! 1. Group by prefix: IPv4 by /24 block, IPv6 by /64 block (a household or one machine usually holds the whole
//!    block and changes address within it) — also how the rate limiter groups IPv6.
//! 2. HMAC-SHA256 with a key derived from the kernel's signing key ([`SecretKey`]), keeping the first 16 bytes,
//!    encoded as base64url (22 characters). Without the key, the IP cannot be recovered from the hash (the IPv4
//!    space is small, and a plain hash can be brute-forced).
//!
//! Two forms:
//!
//! - [`bucket`]: the key changes daily (UTC) — for counters (e.g. "how many submissions from this block
//!   today"). Hashes from two days cannot be linked.
//! - [`fingerprint`]: a fixed key — stored with the record (a comment, a form submission) to ban or group
//!   by sender; the module deletes it itself after its own retention period.
//!
//! Both take a `scope` (module name, or form scope): each scope has its own key (D-153), so the marks of the
//! same person in `forms` and in `comments` differ — being able to read one module's database does not let
//! you link that person to another module. Two records are comparable only within the same scope.
//!
//! Limit: if the root key lives in the database (`AUTOXIDE_SECRET_KEY` unset), anyone with a copy of the
//! database can recompute the key and try every IPv4 block (2^24). The server warns at startup;
//! in production, set a key outside the database.

use crate::time::Timestamp;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::error::Error;
use crate::secret::SecretKey;

/// The address block containing `ip`: /24 for IPv4, /64 for IPv6. An IPv4 address written as IPv6
/// (`::ffff:a.b.c.d`) is treated as IPv4.
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

/// The client that `ip` stands for when counting requests: one IPv4 address, or the /64 block of an IPv6 address
/// (an IPv4 address written as IPv6 counts as IPv4). An IPv6 client usually owns a whole /64 and can change
/// address on every request; an IPv4 address is a real resource, and grouping by /24 would make customers behind
/// a mobile carrier's shared NAT block each other (D-20261125-address-keys). Used by
/// [`crate::throttle::address_key`] and [`client_fingerprint`].
pub fn client(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
        }
        v4 => v4,
    }
}

/// Like [`fingerprint`], for the client [`client`] groups `ip` into (one IPv4 address, or one IPv6 /64)
/// instead of its /24 or /64 block: to tell clients apart where an IPv4 /24 is too coarse.
pub async fn client_fingerprint(
    secret: &SecretKey,
    scope: &str,
    ip: IpAddr,
) -> Result<String, Error> {
    let key = secret.derive(&format!("autoxide/ip-hash/client/v1/{scope}")).await?;
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key)
        .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
    mac.update(client(ip).to_string().as_bytes());
    let bytes = mac.finalize().into_bytes();
    Ok(URL_SAFE_NO_PAD.encode(&bytes[..16]))
}

fn v4_block(v4: Ipv4Addr) -> Ipv4Addr {
    let [a, b, c, _] = v4.octets();
    Ipv4Addr::new(a, b, c, 0)
}

/// The hash of the block containing `ip` in scope `scope`, with a key that changes daily by the UTC date of
/// `now`. Used for counters.
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

/// The fixed hash of the block containing `ip` in scope `scope` (a module name). Stored with the record;
/// the module deletes it after its own retention period.
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
