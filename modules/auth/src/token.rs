//! Access token: JWT (RFC 7519) ký bằng HMAC-SHA256 (`HS256`).
//!
//! Token mang ID người dùng (`sub`), thời điểm cấp (`iat`) và hết hạn (`exp`), bên
//! cấp (`iss`). Máy chủ kiểm chữ ký bằng khoá bí mật, không cần hỏi database —
//! nên token phải sống ngắn: nó không thu hồi được trước khi hết hạn.
//!
//! Tự viết thay vì dùng thư viện `jsonwebtoken`: ta chỉ cần đúng một thuật toán,
//! còn thư viện kéo theo RSA và đường cong elliptic có lỗ hổng chưa vá (D-045).
//! Phần dễ sai nhất của JWT — chấp nhận thuật toán do token tự khai (`alg: none`,
//! đổi sang thuật toán khác) — không thể xảy ra ở đây: chỉ `HS256` được nhận.
//!
//! Ngoài ra file này sinh và băm refresh token (chuỗi ngẫu nhiên, không phải JWT).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
// Một cách làm cho cả chương trình: token ngẫu nhiên và băm SHA-256 của kernel.
pub(crate) use autoxide_kernel::secret::{random_token, sha256};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;
use uuid::Uuid;

use autoxide_kernel::error::Error;

/// Giá trị `iss` của mọi token Autoxide cấp.
const ISSUER: &str = "autoxide";

/// Phần header của mọi token, đã mã hoá sẵn: `{"alg":"HS256","typ":"JWT"}`.
const HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iss: String,
    sub: Uuid,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Deserialize)]
struct Header {
    alg: String,
}

/// Bộ cấp và kiểm access token.
#[derive(Clone)]
pub(crate) struct AccessTokens {
    key: Vec<u8>,
    ttl: Duration,
}

// Tự viết `Debug` để khoá không bao giờ lọt vào log.
impl std::fmt::Debug for AccessTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessTokens").field("ttl", &self.ttl).finish_non_exhaustive()
    }
}

impl AccessTokens {
    pub(crate) fn new(key: Vec<u8>, ttl: Duration) -> Self {
        AccessTokens { key, ttl }
    }

    /// Thời gian sống của một token.
    pub(crate) fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Cấp token cho người dùng `user`, tính từ `now`.
    pub(crate) fn issue(&self, user: Uuid, now: DateTime<Utc>) -> Result<String, Error> {
        let iat = now.timestamp();
        let ttl = i64::try_from(self.ttl.as_secs()).map_err(Error::internal)?;
        let claims = Claims { iss: ISSUER.to_owned(), sub: user, iat, exp: iat + ttl };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).map_err(Error::internal)?);
        let signing_input = format!("{HEADER}.{payload}");
        let signature = URL_SAFE_NO_PAD.encode(self.mac(&signing_input)?.finalize().into_bytes());
        Ok(format!("{signing_input}.{signature}"))
    }

    /// Kiểm `token` tại thời điểm `now`; hợp lệ thì trả ID người dùng.
    pub(crate) fn verify(&self, token: &str, now: DateTime<Utc>) -> Result<Uuid, Error> {
        // Đúng ba phần `header.payload.signature`; chữ ký phủ `header.payload`.
        let (signing_input, signature) = token.rsplit_once('.').ok_or_else(invalid_token)?;
        let (header, payload) = signing_input.split_once('.').ok_or_else(invalid_token)?;
        if payload.contains('.') {
            return Err(invalid_token());
        }
        // Kiểm header TRƯỚC: chỉ nhận đúng HS256, không tin thuật toán token tự khai.
        let header: Header = decode_json(header)?;
        if header.alg != "HS256" {
            return Err(invalid_token());
        }
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| invalid_token())?;
        // `verify_slice` so sánh trong thời gian không đổi: không lộ dần chữ ký đúng
        // qua thời gian trả lời.
        self.mac(signing_input)?.verify_slice(&signature).map_err(|_| invalid_token())?;

        let claims: Claims = decode_json(payload)?;
        if claims.iss != ISSUER {
            return Err(invalid_token());
        }
        if claims.exp <= now.timestamp() {
            return Err(Error::unauthenticated(
                "auth.token_expired",
                "Your session has expired. Sign in again.",
            ));
        }
        Ok(claims.sub)
    }

    fn mac(&self, signing_input: &str) -> Result<HmacSha256, Error> {
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&self.key)
            .map_err(|error| Error::internal(format!("hmac: {error}")))?;
        mac.update(signing_input.as_bytes());
        Ok(mac)
    }
}

fn decode_json<T: serde::de::DeserializeOwned>(part: &str) -> Result<T, Error> {
    let bytes = URL_SAFE_NO_PAD.decode(part).map_err(|_| invalid_token())?;
    serde_json::from_slice(&bytes).map_err(|_| invalid_token())
}

pub(crate) fn invalid_token() -> Error {
    Error::unauthenticated("auth.invalid_token", "Your sign-in is not valid. Sign in again.")
}

/// Băm SHA-256 của refresh token, để lưu. Database chỉ giữ bản băm: lộ bảng cũng
/// không dùng được token. Không cần Argon2 ở đây — token đã ngẫu nhiên 256 bit,
/// không đoán thử được như mật khẩu.
pub(crate) fn hash_refresh_token(token: &str) -> String {
    sha256(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> AccessTokens {
        AccessTokens::new(b"0123456789abcdef0123456789abcdef".to_vec(), Duration::from_secs(900))
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    #[test]
    fn header_da_ma_hoa_dung() {
        assert_eq!(
            String::from_utf8(URL_SAFE_NO_PAD.decode(HEADER).unwrap()).unwrap(),
            r#"{"alg":"HS256","typ":"JWT"}"#
        );
    }

    #[test]
    fn cap_roi_kiem_lai_duoc() {
        let user = Uuid::now_v7();
        let token = tokens().issue(user, now()).unwrap();

        assert_eq!(token.split('.').count(), 3);
        assert_eq!(tokens().verify(&token, now()).unwrap(), user);
    }

    #[test]
    fn het_han_thi_bao_rieng() {
        let token = tokens().issue(Uuid::now_v7(), now()).unwrap();

        let still_valid = tokens().verify(&token, now() + chrono::Duration::seconds(899));
        let expired = tokens().verify(&token, now() + chrono::Duration::seconds(900));

        assert!(still_valid.is_ok());
        assert_eq!(expired.unwrap_err().code(), "auth.token_expired");
    }

    #[test]
    fn sai_khoa_sua_noi_dung_hay_hong_deu_bi_tu_choi() {
        let token = tokens().issue(Uuid::now_v7(), now()).unwrap();
        let other_key = AccessTokens::new(
            b"another-key-another-key-another!!".to_vec(),
            Duration::from_secs(900),
        );
        let (signing_input, signature) = token.rsplit_once('.').unwrap();
        let (_, payload) = signing_input.split_once('.').unwrap();
        // Đổi `sub` sang người khác mà giữ chữ ký cũ.
        let forged_claims =
            format!(r#"{{"iss":"autoxide","sub":"{}","iat":0,"exp":9999999999}}"#, Uuid::now_v7());
        let forged =
            format!("{HEADER}.{}.{signature}", URL_SAFE_NO_PAD.encode(forged_claims.as_bytes()));
        // Token "không chữ ký" kiểu `alg: none`.
        let none_header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
        let unsigned = format!("{none_header}.{payload}.");

        for bad in [
            other_key.issue(Uuid::now_v7(), now()).unwrap(),
            forged,
            unsigned,
            format!("{token}.extra"),
            "not-a-token".to_owned(),
            String::new(),
        ] {
            let error = tokens().verify(&bad, now()).unwrap_err();
            assert_eq!(error.code(), "auth.invalid_token", "{bad}");
        }
    }

    #[test]
    fn refresh_token_ngau_nhien_va_bam_on_dinh() {
        let first = random_token(32).unwrap();
        let second = random_token(32).unwrap();

        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
        assert_eq!(hash_refresh_token(&first), hash_refresh_token(&first));
        assert_ne!(hash_refresh_token(&first), first);
    }

    #[test]
    fn debug_khong_in_khoa() {
        assert!(!format!("{:?}", tokens()).contains("0123456789"));
    }
}
