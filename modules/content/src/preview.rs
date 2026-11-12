//! Link xem trước bản nháp (D-102): token ký HMAC-SHA256 bằng
//! `AUTOXIDE_CONTENT_PREVIEW_SECRET`, chứa ID mục và hạn dùng; không lưu bảng. Chưa cấu hình
//! secret thì tính năng tắt (`content.preview_disabled`).
//!
//! Dạng token: `<base64url(ID 16 byte + hạn dùng 8 byte)>.<base64url(chữ ký)>`.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use uuid::Uuid;

use crate::entries::Entry;
use crate::permissions;
use crate::service::{ContentService, now};

type HmacSha256 = Hmac<Sha256>;

impl ContentService {
    /// Token xem trước mục `id`, hết hạn sau `AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS`. Cần
    /// `content.write` (hay `content.write_own` trên mục của mình); không cần `content.read`:
    /// quyền sửa mục là đủ, như khôi phục revision (D-111).
    pub(crate) async fn preview_token(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(String, DateTime<Utc>), Error> {
        permissions::require_create(principal)?;
        let secret = self.preview_secret()?;
        let entry = self.load_checked(id, None).await?;
        permissions::require_write(principal, &entry)?;
        let expires = now() + chrono::Duration::seconds(i64::from(self.config.preview_ttl_seconds));
        Ok((sign(secret, entry.id, expires)?, expires))
    }

    /// Bản nháp của mục trong token (không cần đăng nhập). Token sai, hết hạn, hay mục không
    /// còn: `content.preview_token_invalid`.
    pub(crate) async fn preview(&self, token: &str) -> Result<Entry, Error> {
        let secret = self.preview_secret()?;
        let (payload, signature) = token.split_once('.').ok_or_else(invalid)?;
        let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| invalid())?;
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| invalid())?;
        // `verify_slice` so sánh trong thời gian không đổi: không lộ dần chữ ký đúng.
        mac(secret, &payload)?.verify_slice(&signature).map_err(|_| invalid())?;
        let (id, expires) = payload.split_at_checked(16).ok_or_else(invalid)?;
        let id = Uuid::from_slice(id).map_err(|_| invalid())?;
        let expires = i64::from_be_bytes(expires.try_into().map_err(|_| invalid())?);
        if expires < now().timestamp() {
            return Err(invalid());
        }
        use sea_orm::EntityTrait as _;
        crate::entity::entry::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(invalid)
    }

    fn preview_secret(&self) -> Result<&str, Error> {
        self.config.preview_secret.as_deref().ok_or_else(|| {
            Error::unavailable(
                "content.preview_disabled",
                "Draft previews are off: the server has no AUTOXIDE_CONTENT_PREVIEW_SECRET.",
            )
        })
    }
}

/// Token cho mục `id`, hết hạn lúc `expires`.
pub(crate) fn sign(secret: &str, id: Uuid, expires: DateTime<Utc>) -> Result<String, Error> {
    let mut payload = id.as_bytes().to_vec();
    payload.extend(expires.timestamp().to_be_bytes());
    let signature = mac(secret, &payload)?.finalize().into_bytes();
    Ok(format!("{}.{}", URL_SAFE_NO_PAD.encode(&payload), URL_SAFE_NO_PAD.encode(signature)))
}

fn mac(secret: &str, payload: &[u8]) -> Result<HmacSha256, Error> {
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(secret.as_bytes())
        .map_err(|error| Error::internal(format!("hmac: {error}")))?;
    mac.update(payload);
    Ok(mac)
}

fn invalid() -> Error {
    Error::not_found(
        "content.preview_token_invalid",
        "This preview link is invalid or has expired. Ask for a new one.",
    )
}
