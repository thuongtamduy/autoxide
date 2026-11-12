//! Token form do máy chủ ký: website lấy lúc HIỂN THỊ form (không nhúng trong HTML đã cache),
//! gửi kèm bài. Chứa phạm vi (form nào), lúc phát và một nonce ngẫu nhiên.
//!
//! Dạng: `base64url(độ dài phạm vi 1 byte ‖ phạm vi ‖ lúc phát i64 ‖ nonce 16 byte) .
//! base64url(HMAC-SHA256)`, ký bằng khoá dẫn từ khoá của kernel (nhãn [`PURPOSE`]).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::error::Error;

/// Nhãn dẫn khoá ký token form.
pub(crate) const PURPOSE: &str = "autoxide/antispam/form-token/v1";

/// Nội dung đã kiểm chữ ký của một token form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormToken {
    pub scope: String,
    pub issued_at: DateTime<Utc>,
    /// Nonce (base64url) — khoá idempotency của bài gửi.
    pub nonce: String,
}

pub(crate) fn sign(
    key: &[u8; 32],
    scope: &str,
    issued_at: DateTime<Utc>,
    nonce: &[u8; 16],
) -> Result<String, Error> {
    let scope_len = u8::try_from(scope.len()).map_err(|_| Error::internal("scope too long"))?;
    let mut payload = vec![scope_len];
    payload.extend(scope.as_bytes());
    payload.extend(issued_at.timestamp().to_be_bytes());
    payload.extend(nonce);
    let signature = mac(key, &payload)?.finalize().into_bytes();
    Ok(format!("{}.{}", URL_SAFE_NO_PAD.encode(&payload), URL_SAFE_NO_PAD.encode(signature)))
}

/// Kiểm chữ ký và đọc token; `None` khi sai dạng hay sai chữ ký.
pub(crate) fn open(key: &[u8; 32], token: &str) -> Option<FormToken> {
    let (payload, signature) = token.trim().split_once('.')?;
    let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
    // `verify_slice` so sánh trong thời gian không đổi.
    mac(key, &payload).ok()?.verify_slice(&signature).ok()?;
    let (&scope_len, rest) = payload.split_first()?;
    let (scope, rest) = rest.split_at_checked(usize::from(scope_len))?;
    let (issued, nonce) = rest.split_at_checked(8)?;
    if nonce.len() != 16 {
        return None;
    }
    let issued_at = DateTime::from_timestamp(i64::from_be_bytes(issued.try_into().ok()?), 0)?;
    Some(FormToken {
        scope: String::from_utf8(scope.to_vec()).ok()?,
        issued_at,
        nonce: URL_SAFE_NO_PAD.encode(nonce),
    })
}

/// Nonce dạng chữ (base64url), như trong [`FormToken::nonce`].
pub(crate) fn encode_nonce(nonce: &[u8; 16]) -> String {
    URL_SAFE_NO_PAD.encode(nonce)
}

fn mac(key: &[u8; 32], payload: &[u8]) -> Result<Hmac<Sha256>, Error> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
    mac.update(payload);
    Ok(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ky_va_doc_lai() {
        let key = [3u8; 32];
        let at = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let token = sign(&key, "forms:contact", at, &[9u8; 16]).unwrap();
        let opened = open(&key, &token).unwrap();
        assert_eq!((opened.scope.as_str(), opened.issued_at), ("forms:contact", at));
        assert_eq!(opened.nonce, URL_SAFE_NO_PAD.encode([9u8; 16]));
        assert!(open(&[4u8; 32], &token).is_none(), "other key");
        let mut tampered = token.clone();
        tampered.replace_range(0..1, if token.starts_with('A') { "B" } else { "A" });
        assert!(open(&key, &tampered).is_none());
        assert!(open(&key, "garbage").is_none());
        assert!(open(&key, "a.b").is_none());
    }
}
