//! Link xem trước bản nháp (D-102): token ký HMAC-SHA256 bằng
//! `AUTOXIDE_CONTENT_PREVIEW_SECRET`, chứa ID mục và hạn dùng; không lưu bảng. Chưa cấu hình
//! secret thì tính năng tắt (`content.preview_disabled`).
//!
//! Dạng token: `<base64url(ID 16 byte + hạn dùng 8 byte [+ ID khách 16 byte])>.<base64url(chữ ký)>`.
//!
//! Theo khách (bước 33): token của khách mặc định giữ đúng dạng và khoá cũ (link đã phát vẫn
//! mở được). Token của khách khác mang ID khách trong phần đã ký — mở được cả trên tên miền chính
//! không header — và ký bằng khoá riêng của khách đó, dẫn xuất từ secret như
//! `SecretKey::derive_for`: token của khách này mang sang khách khác thì như token sai.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, ScopeExt, TenantContext, TenantId, ensure_serving};
use autoxide_kernel::time::Timestamp;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use jiff::SignedDuration;
use sha2::Sha256;
use uuid::Uuid;

use crate::entries::Entry;
use crate::permissions;
use crate::service::{ContentService, now};

type HmacSha256 = Hmac<Sha256>;

impl ContentService {
    /// Token xem trước mục `id` của khách `tenant`, hết hạn sau
    /// `AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS`. Cần `content.write` (hay `content.write_own` trên
    /// mục của mình); không cần `content.read`: quyền sửa mục là đủ, như khôi phục revision
    /// (D-111).
    pub(crate) async fn preview_token(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(String, Timestamp), Error> {
        permissions::require_create(principal)?;
        let secret = self.preview_secret()?;
        let entry = self.load_checked(tenant, id, None).await?;
        permissions::require_write(principal, &entry)?;
        permissions::require_org(principal, tenant, &entry, permissions::Need::Write).await?;
        let expires = now() + SignedDuration::from_secs(i64::from(self.config.preview_ttl_seconds));
        Ok((sign(secret, tenant.id(), entry.id, expires)?, expires))
    }

    /// Khách và bản nháp của mục trong token (không cần đăng nhập). `request`: khách của request,
    /// nếu xác định được (không có: khách ghi trong token, hay khách mặc định). Token sai, hết
    /// hạn, của khách khác khách của request, hay mục không còn: `content.preview_token_invalid`.
    pub(crate) async fn preview(
        &self,
        request: Option<&TenantContext>,
        token: &str,
    ) -> Result<(TenantContext, Entry), Error> {
        let secret = self.preview_secret()?;
        let (payload, signature) = token.split_once('.').ok_or_else(invalid)?;
        let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| invalid())?;
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| invalid())?;
        let (head, owner) = match payload.len() {
            24 => (&payload[..], None),
            40 => {
                let (head, owner) = payload.split_at(24);
                (head, Some(TenantId::from_uuid(Uuid::from_slice(owner).map_err(|_| invalid())?)))
            }
            _ => return Err(invalid()),
        };
        // Request đã nêu khách (tên miền, header, credential) mà token của khách khác: token sai.
        if let (Some(owner), Some(request)) = (owner, request)
            && owner != request.id()
        {
            return Err(invalid());
        }
        let tenant = match owner {
            None => request.cloned().unwrap_or_else(|| self.databases.default_tenant()),
            // Khách ghi trong token: tra qua sổ. Không có hay không phục vụ được: như token sai —
            // không lộ khách nào tồn tại.
            Some(owner) => match self.databases.tenant(owner).await {
                Ok(tenant) if ensure_serving(&tenant).is_ok() => tenant,
                Ok(_) => return Err(invalid()),
                Err(error) if error.kind() == ErrorKind::NotFound => return Err(invalid()),
                Err(error) => return Err(error),
            },
        };
        // `verify_slice` so sánh trong thời gian không đổi: không lộ dần chữ ký đúng.
        mac(secret, tenant.id(), &payload)?.verify_slice(&signature).map_err(|_| invalid())?;
        // Token không ghi khách chỉ khách mặc định phát ra (khách khác luôn ghi khách).
        if owner.is_none() && tenant.id() != DEFAULT_TENANT {
            return Err(invalid());
        }
        let (id, expires) = head.split_at_checked(16).ok_or_else(invalid)?;
        let id = Uuid::from_slice(id).map_err(|_| invalid())?;
        let expires = i64::from_be_bytes(expires.try_into().map_err(|_| invalid())?);
        if expires < now().as_unix_seconds() {
            return Err(invalid());
        }
        use sea_orm::EntityTrait as _;
        let entry = crate::entity::entry::Entity::find_by_id(id)
            .scoped(&tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(invalid)?;
        Ok((tenant, entry))
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

/// Token cho mục `id` của khách `tenant`, hết hạn lúc `expires`.
pub(crate) fn sign(
    secret: &str,
    tenant: TenantId,
    id: Uuid,
    expires: Timestamp,
) -> Result<String, Error> {
    let mut payload = id.as_bytes().to_vec();
    payload.extend(expires.as_unix_seconds().to_be_bytes());
    if tenant != DEFAULT_TENANT {
        payload.extend(tenant.as_uuid().as_bytes());
    }
    let signature = mac(secret, tenant, &payload)?.finalize().into_bytes();
    Ok(format!("{}.{}", URL_SAFE_NO_PAD.encode(&payload), URL_SAFE_NO_PAD.encode(signature)))
}

/// HMAC của `payload` bằng khoá của khách `tenant`: khách mặc định dùng thẳng secret (như trước
/// bước 33); khách khác dùng HMAC(secret, nhãn có ID khách) — cách `SecretKey::derive_for` làm.
fn mac(secret: &str, tenant: TenantId, payload: &[u8]) -> Result<HmacSha256, Error> {
    let hmac = |key: &[u8]| {
        <HmacSha256 as KeyInit>::new_from_slice(key)
            .map_err(|error| Error::internal(format!("hmac: {error}")))
    };
    let mut mac = if tenant == DEFAULT_TENANT {
        hmac(secret.as_bytes())?
    } else {
        let mut derive = hmac(secret.as_bytes())?;
        // Byte 0 ngăn cách: không ghép (mục đích, khách) khác nhau thành cùng một nhãn.
        derive.update(format!("content.preview\0tenant\0{tenant}").as_bytes());
        hmac(&derive.finalize().into_bytes())?
    };
    mac.update(payload);
    Ok(mac)
}

fn invalid() -> Error {
    Error::not_found(
        "content.preview_token_invalid",
        "This preview link is invalid or has expired. Ask for a new one.",
    )
}
