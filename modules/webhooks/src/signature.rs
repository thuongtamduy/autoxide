//! Chữ ký theo chuẩn Standard Webhooks (<https://www.standardwebhooks.com>): bên nhận
//! dùng thư viện có sẵn của chuẩn để kiểm, không phải tự viết.
//!
//! Mỗi request mang ba header:
//!
//! - `webhook-id`: id của lần gửi (giống nhau qua mọi lần thử lại — bên nhận dùng để bỏ
//!   bản trùng);
//! - `webhook-timestamp`: giây Unix lúc ký (bên nhận từ chối request quá cũ);
//! - `webhook-signature`: `v1,<base64 của HMAC-SHA256(secret, "<id>.<timestamp>.<thân>")>`.

use autoxide_kernel::error::Error;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Tiền tố của secret theo chuẩn.
const SECRET_PREFIX: &str = "whsec_";

/// Số byte ngẫu nhiên của một secret (chuẩn đòi 24 tới 64).
const SECRET_BYTES: usize = 32;

/// Secret mới cho một endpoint: `whsec_` + base64 của 32 byte ngẫu nhiên.
pub(crate) fn new_secret() -> Result<String, Error> {
    let mut bytes = [0u8; SECRET_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
    Ok(format!("{SECRET_PREFIX}{}", STANDARD.encode(bytes)))
}

/// Giá trị header `webhook-signature` cho thân `body` của lần gửi `id` lúc `timestamp`.
pub(crate) fn sign(secret: &str, id: &str, timestamp: i64, body: &str) -> Result<String, Error> {
    let key = STANDARD
        .decode(secret.strip_prefix(SECRET_PREFIX).unwrap_or(secret))
        .map_err(|error| Error::internal(format!("webhook secret is not base64: {error}")))?;
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key)
        .map_err(|error| Error::internal(format!("hmac: {error}")))?;
    mac.update(format!("{id}.{timestamp}.").as_bytes());
    mac.update(body.as_bytes());
    Ok(format!("v1,{}", STANDARD.encode(mac.finalize().into_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ví dụ trong bộ test chính thức của Standard Webhooks: chữ ký phải khớp từng byte
    /// để mọi thư viện của chuẩn kiểm được.
    #[test]
    fn khop_vi_du_cua_chuan() {
        let secret = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
        let id = "msg_p5jXN8AQM9LWM0D4loKWxJek";
        let body = r#"{"test": 2432232314}"#;
        assert_eq!(
            sign(secret, id, 1614265330, body).unwrap(),
            "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE="
        );
    }

    #[test]
    fn secret_moi_dung_dang_va_khac_nhau() {
        let first = new_secret().unwrap();
        assert!(first.starts_with("whsec_"));
        assert_eq!(STANDARD.decode(&first[6..]).unwrap().len(), SECRET_BYTES);
        assert_ne!(first, new_secret().unwrap());
    }
}
