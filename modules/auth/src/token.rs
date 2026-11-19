//! Access token: JWT (RFC 7519) ký bằng HMAC-SHA256 (`HS256`).
//!
//! Token mang ID người dùng (`sub`), khách của người dùng (`tid`, bước 33), thời điểm cấp
//! (`iat`) và hết hạn (`exp`), bên cấp (`iss`). Máy chủ kiểm chữ ký bằng khoá bí mật, không cần
//! hỏi database — nên token phải sống ngắn: nó không thu hồi được trước khi hết hạn.
//!
//! **Khoá ký theo khách (bước 33, D-285):** khách mặc định ký bằng đúng khoá cấu hình
//! (`AUTOXIDE_AUTH_TOKEN_SECRET`) như trước — token cấp trước khi nâng cấp vẫn dùng được; khách
//! khác ký bằng khoá dẫn xuất `HMAC-SHA256(khoá cấu hình, "auth.access_token\0tenant\0<id>")`,
//! cùng cách nhãn với `SecretKey::derive_for` của kernel. Khoá cấu hình không lấy từ `SecretKey`
//! nên dẫn xuất ở đây. Đổi `tid` của token sang khách khác mà giữ chữ ký thì chữ ký không khớp
//! khoá của khách kia: token bị từ chối. Token không có `tid` (cấp trước bước 33) thuộc khách
//! mặc định.
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
use autoxide_kernel::tenancy::{
    DEFAULT_TENANT, DatabaseRegistry, TenantContext, TenantId, resolve_link_token,
};

/// Giá trị `iss` của mọi token Autoxide cấp.
const ISSUER: &str = "autoxide";

/// Mục đích của khoá ký access token, trong nhãn dẫn xuất khoá của khách (xem đầu file).
const PURPOSE: &str = "auth.access_token";

/// Phần header của mọi token, đã mã hoá sẵn: `{"alg":"HS256","typ":"JWT"}`.
const HEADER: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iss: String,
    sub: Uuid,
    /// Khách của người dùng. Token cấp trước bước 33 không có: khách mặc định.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tid: Option<Uuid>,
    iat: i64,
    exp: i64,
    /// Lần đăng nhập hay xác nhận lại gần nhất của phiên (giây Unix, như `auth_time` của OpenID
    /// Connect): việc nhạy cảm đòi nó còn mới (bước 54 P5). Token cũ không có.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_time: Option<i64>,
    /// Phiên (họ refresh token) cấp token này: `POST /me/reauth` làm mới đúng phiên đó.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sid: Option<Uuid>,
}

/// Nội dung đã kiểm của một access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Verified {
    pub tenant: TenantId,
    pub user: Uuid,
    /// Phiên cấp token; `None` với token cũ.
    pub session: Option<Uuid>,
    /// Lần xác thực gần nhất của phiên; `None` với token cũ.
    pub authenticated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct Header {
    alg: String,
}

/// Bộ cấp và kiểm access token.
#[derive(Clone)]
pub(crate) struct AccessTokens {
    key: Vec<u8>,
    /// Khoá cũ (`AUTOXIDE_AUTH_TOKEN_SECRET_PREVIOUS`): chỉ để KIỂM token ký trước khi đổi khoá.
    previous: Vec<Vec<u8>>,
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
        AccessTokens { key, previous: Vec::new(), ttl }
    }

    /// Nhận thêm token ký bằng các khoá cũ `previous` trong lúc đổi khoá (bước 39): token mới vẫn
    /// ký bằng khoá hiện tại, nên đổi khoá không đăng xuất ai. Định dạng token không đổi (không
    /// thêm `kid`): token đang nằm ở client vẫn đọc được.
    pub(crate) fn with_previous(mut self, previous: Vec<Vec<u8>>) -> Self {
        self.previous = previous;
        self
    }

    /// Thời gian sống của một token.
    pub(crate) fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Cấp token cho người dùng `user` của khách `tenant`, tính từ `now`, thuộc phiên `session`
    /// xác thực lần gần nhất lúc `authenticated_at`.
    pub(crate) fn issue(
        &self,
        tenant: TenantId,
        user: Uuid,
        now: DateTime<Utc>,
        session: Uuid,
        authenticated_at: DateTime<Utc>,
    ) -> Result<String, Error> {
        let iat = now.timestamp();
        let ttl = i64::try_from(self.ttl.as_secs()).map_err(Error::internal)?;
        let claims = Claims {
            iss: ISSUER.to_owned(),
            sub: user,
            tid: Some(tenant.as_uuid()),
            iat,
            exp: iat + ttl,
            auth_time: Some(authenticated_at.timestamp()),
            sid: Some(session),
        };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).map_err(Error::internal)?);
        let signing_input = format!("{HEADER}.{payload}");
        let signature =
            URL_SAFE_NO_PAD.encode(mac(&self.key, tenant, &signing_input)?.finalize().into_bytes());
        Ok(format!("{signing_input}.{signature}"))
    }

    /// Kiểm `token` tại thời điểm `now`; hợp lệ thì trả (khách, ID người dùng). Khách đọc từ
    /// claim `tid` TRƯỚC khi kiểm chữ ký — để chọn khoá — rồi chữ ký phải khớp khoá của đúng khách
    /// đó: `tid` giả không qua được.
    pub(crate) fn verify(&self, token: &str, now: DateTime<Utc>) -> Result<Verified, Error> {
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
        // Nội dung chưa được tin: chỉ đọc `tid` để chọn khoá kiểm chữ ký.
        let claims: Claims = decode_json(payload)?;
        let tenant = claims.tid.map_or(DEFAULT_TENANT, TenantId::from_uuid);
        // `verify_slice` so sánh trong thời gian không đổi: không lộ dần chữ ký đúng
        // qua thời gian trả lời. Thử khoá hiện tại rồi từng khoá cũ (đang đổi khoá).
        let mut valid = false;
        for key in std::iter::once(&self.key).chain(&self.previous) {
            if mac(key, tenant, signing_input)?.verify_slice(&signature).is_ok() {
                valid = true;
                break;
            }
        }
        if !valid {
            return Err(invalid_token());
        }

        if claims.iss != ISSUER {
            return Err(invalid_token());
        }
        if claims.exp <= now.timestamp() {
            return Err(Error::unauthenticated(
                "auth.token_expired",
                "Your session has expired. Sign in again.",
            ));
        }
        Ok(Verified {
            tenant,
            user: claims.sub,
            session: claims.sid,
            authenticated_at: claims
                .auth_time
                .and_then(|seconds| DateTime::from_timestamp(seconds, 0)),
        })
    }
}

/// HMAC đã nạp `signing_input`, với khoá ký của khách `tenant` dẫn từ `key` (xem đầu file).
fn mac(key: &[u8], tenant: TenantId, signing_input: &str) -> Result<HmacSha256, Error> {
    let mut mac = if tenant == DEFAULT_TENANT {
        hmac(key)?
    } else {
        // Byte 0 ngăn cách: không ghép (mục đích, khách) khác nhau thành cùng một nhãn.
        let mut derive = hmac(key)?;
        derive.update(format!("{PURPOSE}\0tenant\0{tenant}").as_bytes());
        hmac(&derive.finalize().into_bytes())?
    };
    mac.update(signing_input.as_bytes());
    Ok(mac)
}

fn hmac(key: &[u8]) -> Result<HmacSha256, Error> {
    <HmacSha256 as KeyInit>::new_from_slice(key)
        .map_err(|error| Error::internal(format!("hmac: {error}")))
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

/// Khách và phần thô của token một lần mà client gửi lại (refresh token, `mfa_token`, token
/// trong mail): token cấp ra mang khách bằng `autoxide_kernel::tenancy::link_token` (khách mặc
/// định giữ dạng cũ, khách khác `t<ID khách>.<phần thô>`), nên tra được ở database của đúng khách
/// TRƯỚC khi tra bản băm của phần thô — kể cả khi request tới tên miền chính không header
/// (`request` là `None`). `None`: token sai dạng, khách không có / không phục vụ được, hay khách
/// của request (header, tên miền) khác khách ghi trong token — bên gọi trả lỗi "token sai" như
/// thường.
pub(crate) async fn resolve<'t>(
    databases: &DatabaseRegistry,
    request: Option<&TenantContext>,
    token: &'t str,
) -> Result<Option<(TenantContext, &'t str)>, Error> {
    let Some((tenant, raw)) = resolve_link_token(databases, request, token).await? else {
        return Ok(None);
    };
    if request.is_some_and(|request| request.id() != tenant.id()) {
        return Ok(None);
    }
    Ok(Some((tenant, raw)))
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

    /// Ký `claims` (JSON) bằng `key` như bản trước bước 33 (khoá cấu hình, không theo khách).
    fn sign_with(key: &[u8], claims: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(claims.as_bytes());
        let signing_input = format!("{HEADER}.{payload}");
        let mut mac = hmac(key).unwrap();
        mac.update(signing_input.as_bytes());
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn cap_roi_kiem_lai_duoc() {
        let user = Uuid::now_v7();
        let token = tokens().issue(DEFAULT_TENANT, user, now(), Uuid::nil(), now()).unwrap();

        assert_eq!(token.split('.').count(), 3);
        assert_eq!(
            tokens().verify(&token, now()).map(|v| (v.tenant, v.user)).unwrap(),
            (DEFAULT_TENANT, user)
        );
        let (_, payload) = token.split_once('.').unwrap();
        let claims: Claims = decode_json(payload.split_once('.').unwrap().0).unwrap();
        assert_eq!(claims.tid, Some(DEFAULT_TENANT.as_uuid()));
    }

    /// Khách khác ký bằng khoá riêng; đổi `tid` mà giữ chữ ký thì bị từ chối; token cũ không có
    /// `tid` (ký bằng khoá cấu hình) thuộc khách mặc định và vẫn dùng được.
    #[test]
    fn khoa_theo_khach_va_token_cu() {
        let other = TenantId::from_uuid(Uuid::now_v7());
        let user = Uuid::now_v7();
        let token = tokens().issue(other, user, now(), Uuid::nil(), now()).unwrap();
        assert_eq!(
            tokens().verify(&token, now()).map(|v| (v.tenant, v.user)).unwrap(),
            (other, user)
        );
        // Khoá của khách khác khác khoá cấu hình: cùng nội dung, khác chữ ký.
        let in_default = tokens().issue(DEFAULT_TENANT, user, now(), Uuid::nil(), now()).unwrap();
        assert_ne!(token.rsplit_once('.').unwrap().1, in_default.rsplit_once('.').unwrap().1);

        // `tid` đổi sang khách mặc định (hay khách khác), chữ ký giữ nguyên.
        let signature = token.rsplit_once('.').unwrap().1;
        for tid in [DEFAULT_TENANT, TenantId::from_uuid(Uuid::now_v7())] {
            let claims = format!(
                r#"{{"iss":"autoxide","sub":"{user}","tid":"{tid}","iat":0,"exp":9999999999}}"#
            );
            let forged =
                format!("{HEADER}.{}.{signature}", URL_SAFE_NO_PAD.encode(claims.as_bytes()));
            assert_eq!(tokens().verify(&forged, now()).unwrap_err().code(), "auth.invalid_token");
        }

        // Token cấp trước bước 33: không `tid`, ký bằng khoá cấu hình.
        let legacy = sign_with(
            b"0123456789abcdef0123456789abcdef",
            &format!(r#"{{"iss":"autoxide","sub":"{user}","iat":0,"exp":9999999999}}"#),
        );
        assert_eq!(
            tokens().verify(&legacy, now()).map(|v| (v.tenant, v.user)).unwrap(),
            (DEFAULT_TENANT, user)
        );
    }

    #[test]
    fn het_han_thi_bao_rieng() {
        let token =
            tokens().issue(DEFAULT_TENANT, Uuid::now_v7(), now(), Uuid::nil(), now()).unwrap();

        let still_valid = tokens().verify(&token, now() + chrono::Duration::seconds(899));
        let expired = tokens().verify(&token, now() + chrono::Duration::seconds(900));

        assert!(still_valid.is_ok());
        assert_eq!(expired.unwrap_err().code(), "auth.token_expired");
    }

    fn keyed(key: &[u8]) -> AccessTokens {
        AccessTokens::new(key.to_vec(), Duration::from_secs(900))
    }

    const OLD: &[u8] = b"old-key-old-key-old-key-old-key!";
    const NEW: &[u8] = b"new-key-new-key-new-key-new-key!";

    /// Đổi khoá: token ký bằng khoá cũ vẫn hợp lệ khi khoá cũ nằm trong danh sách chuyển tiếp — ở
    /// khách mặc định lẫn khách khác (khoá dẫn theo khách).
    #[test]
    fn token_ky_bang_khoa_cu_van_hop_le_trong_chuyen_tiep() {
        let user = Uuid::now_v7();
        let other = TenantId::from_uuid(Uuid::now_v7());
        let rotated = keyed(NEW).with_previous(vec![OLD.to_vec()]);
        for tenant in [DEFAULT_TENANT, other] {
            let old = keyed(OLD).issue(tenant, user, now(), Uuid::nil(), now()).unwrap();
            assert_eq!(
                rotated.verify(&old, now()).map(|v| (v.tenant, v.user)).unwrap(),
                (tenant, user)
            );
            // Bỏ khoá cũ khỏi danh sách thì token cũ hết hiệu lực.
            assert_eq!(keyed(NEW).verify(&old, now()).unwrap_err().code(), "auth.invalid_token");
        }
    }

    #[test]
    fn token_ky_bang_khoa_la_bi_tu_choi() {
        let rotated = keyed(NEW).with_previous(vec![OLD.to_vec()]);
        let stranger = keyed(b"someone-elses-key-someone-elses!").issue(
            DEFAULT_TENANT,
            Uuid::now_v7(),
            now(),
            Uuid::nil(),
            now(),
        );
        assert_eq!(
            rotated.verify(&stranger.unwrap(), now()).unwrap_err().code(),
            "auth.invalid_token"
        );
    }

    #[test]
    fn token_moi_ky_bang_khoa_moi() {
        let rotated = keyed(NEW).with_previous(vec![OLD.to_vec()]);
        let token =
            rotated.issue(DEFAULT_TENANT, Uuid::now_v7(), now(), Uuid::nil(), now()).unwrap();
        assert!(keyed(NEW).verify(&token, now()).is_ok());
        assert!(keyed(OLD).verify(&token, now()).is_err());
    }

    #[test]
    fn sai_khoa_sua_noi_dung_hay_hong_deu_bi_tu_choi() {
        let token =
            tokens().issue(DEFAULT_TENANT, Uuid::now_v7(), now(), Uuid::nil(), now()).unwrap();
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
            other_key.issue(DEFAULT_TENANT, Uuid::now_v7(), now(), Uuid::nil(), now()).unwrap(),
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
