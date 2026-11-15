//! Mã hoá secret khi lưu (bước 33, D-257): secret module cần đọc lại được (secret TOTP, secret ký
//! webhook, khoá kết nối đối tác của từng khách) lưu dạng
//! `sealed:v1:<key_id>:<base64url(nonce || ciphertext || tag)>`.
//!
//! - Thuật toán: AES-256-GCM (AEAD) của `ring`; nonce 96 bit ngẫu nhiên mỗi lần mã hoá.
//! - Khoá: `SecretKey::derive("autoxide/kernel/seal/v1")` của khoá gốc — không bao giờ nằm cạnh dữ
//!   liệu khi `AUTOXIDE_SECRET_KEY` đã đặt. `key_id` = 8 ký tự hex đầu của SHA-256 khoá: biết giá
//!   trị được mã hoá bằng khoá nào mà không lộ khoá.
//! - Dữ liệu đi kèm (AAD) = mục đích + khách + định danh dòng: chép giá trị đã mã hoá sang dòng
//!   khác, khách khác hay mục đích khác thì không mở được.
//! - Xoay khoá: khoá cũ vẫn mở được; [`Sealer::needs_reseal`] cho biết giá trị nào cần mã hoá lại
//!   bằng khoá hiện tại (`autoxide secrets reseal`).
//! - Giá trị không có tiền tố `sealed:` là chữ thường của bản cũ: [`Sealer::open`] trả nguyên văn
//!   (chỉ trong lúc chuyển tiếp), [`Sealer::needs_reseal`] là `true`.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::Error;
use crate::tenancy::TenantId;

/// Nhãn dẫn khoá mã hoá từ khoá gốc.
pub(crate) const PURPOSE: &str = "autoxide/kernel/seal/v1";

const PREFIX: &str = "sealed:v1:";

/// Bộ mã hoá secret: khoá hiện tại và các khoá cũ (theo `key_id`). Clone rẻ.
#[derive(Clone)]
pub struct Sealer {
    // Khoá đầu tiên là khoá hiện tại.
    keys: Arc<Vec<(String, LessSafeKey)>>,
}

impl std::fmt::Debug for Sealer {
    // Không bao giờ in khoá.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self.keys.iter().map(|(id, _)| id.as_str()).collect();
        f.debug_struct("Sealer").field("key_ids", &ids).finish()
    }
}

impl Sealer {
    /// Bộ mã hoá với khoá hiện tại `current` và các khoá cũ `previous` (mỗi khoá 32 byte).
    pub(crate) fn new(current: [u8; 32], previous: Vec<[u8; 32]>) -> Result<Self, Error> {
        let mut keys: Vec<(String, LessSafeKey)> = Vec::new();
        for key in std::iter::once(current).chain(previous) {
            let key = Zeroizing::new(key);
            let id = key_id(key.as_slice());
            if keys.iter().any(|(known, _)| *known == id) {
                continue;
            }
            let unbound = UnboundKey::new(&AES_256_GCM, key.as_slice())
                .map_err(|_| Error::internal("cannot load the sealing key"))?;
            keys.push((id, LessSafeKey::new(unbound)));
        }
        Ok(Sealer { keys: Arc::new(keys) })
    }

    /// `key_id` của khoá hiện tại.
    pub fn current_key_id(&self) -> &str {
        self.keys.first().map_or("", |(id, _)| id.as_str())
    }

    /// Mã hoá `plaintext` của khách `tenant`, cho mục đích `purpose` (vd `webhooks.endpoint_secret`),
    /// gắn với dòng `row` (vd ID của dòng).
    pub fn seal(
        &self,
        tenant: TenantId,
        purpose: &str,
        row: &str,
        plaintext: &[u8],
    ) -> Result<String, Error> {
        let (id, key) =
            self.keys.first().ok_or_else(|| Error::internal("the sealer has no key"))?;
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce)
            .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
        let mut data = plaintext.to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(aad(tenant, purpose, row)),
            &mut data,
        )
        .map_err(|_| Error::internal("cannot seal the value"))?;
        let mut packed = nonce.to_vec();
        packed.extend_from_slice(&data);
        Ok(format!("{PREFIX}{id}:{}", URL_SAFE_NO_PAD.encode(packed)))
    }

    /// Mở giá trị `sealed` đã mã hoá bằng [`seal`](Self::seal) với cùng khách, mục đích và dòng.
    /// Giá trị chữ thường của bản cũ (không tiền tố `sealed:`) trả nguyên văn.
    pub fn open(
        &self,
        tenant: TenantId,
        purpose: &str,
        row: &str,
        sealed: &str,
    ) -> Result<Zeroizing<Vec<u8>>, Error> {
        if !is_sealed(sealed) {
            return Ok(Zeroizing::new(sealed.as_bytes().to_vec()));
        }
        let rest = sealed.strip_prefix(PREFIX).ok_or_else(|| unreadable(purpose))?;
        let (id, packed) = rest.split_once(':').ok_or_else(|| unreadable(purpose))?;
        let key = self.keys.iter().find(|(known, _)| known == id).map(|(_, key)| key);
        let key = key.ok_or_else(|| {
            Error::internal(format!(
                "a stored `{purpose}` secret was sealed with key `{id}`, which is not configured: \
                 put the old AUTOXIDE_SECRET_KEY in AUTOXIDE_SECRET_KEY_PREVIOUS"
            ))
        })?;
        let packed = URL_SAFE_NO_PAD.decode(packed).map_err(|_| unreadable(purpose))?;
        if packed.len() < NONCE_LEN {
            return Err(unreadable(purpose));
        }
        let (nonce, data) = packed.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| unreadable(purpose))?;
        let mut data = Zeroizing::new(data.to_vec());
        let length = key
            .open_in_place(nonce, Aad::from(aad(tenant, purpose, row)), data.as_mut_slice())
            .map_err(|_| unreadable(purpose))?
            .len();
        data.truncate(length);
        Ok(data)
    }

    /// Giá trị cần mã hoá lại bằng khoá hiện tại: chữ thường cũ, hay mã hoá bằng khoá cũ.
    pub fn needs_reseal(&self, sealed: &str) -> bool {
        let current = format!("{PREFIX}{}:", self.current_key_id());
        !sealed.starts_with(&current)
    }
}

/// Giá trị đã được mã hoá (có tiền tố `sealed:`), dù bằng khoá nào.
pub fn is_sealed(value: &str) -> bool {
    value.starts_with("sealed:")
}

fn aad(tenant: TenantId, purpose: &str, row: &str) -> Vec<u8> {
    // Byte 0 ngăn cách: không ghép hai bộ (mục đích, dòng) khác nhau thành cùng một chuỗi.
    format!("{purpose}\0{tenant}\0{row}").into_bytes()
}

/// 8 ký tự hex đầu của SHA-256 khoá.
fn key_id(key: &[u8]) -> String {
    Sha256::digest(key).iter().take(4).map(|byte| format!("{byte:02x}")).collect()
}

fn unreadable(purpose: &str) -> Error {
    Error::internal(format!(
        "a stored `{purpose}` secret cannot be opened: it was changed, copied from another row or \
         tenant, or sealed with another key"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenancy::DEFAULT_TENANT;

    fn key(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn ma_hoa_mo_lai_va_aad_khac_thi_hong() {
        let sealer = Sealer::new(key(1), Vec::new()).unwrap();
        let sealed = sealer.seal(DEFAULT_TENANT, "demo.secret", "row-1", b"hunter2").unwrap();
        assert!(sealed.starts_with(&format!("sealed:v1:{}:", sealer.current_key_id())));
        assert_eq!(sealer.current_key_id().len(), 8);
        assert_ne!(
            sealed,
            sealer.seal(DEFAULT_TENANT, "demo.secret", "row-1", b"hunter2").unwrap()
        );
        let opened = sealer.open(DEFAULT_TENANT, "demo.secret", "row-1", &sealed).unwrap();
        assert_eq!(opened.as_slice(), b"hunter2");
        let other = TenantId::from_uuid(uuid::Uuid::now_v7());
        assert!(sealer.open(other, "demo.secret", "row-1", &sealed).is_err(), "other tenant");
        assert!(sealer.open(DEFAULT_TENANT, "demo.secret", "row-2", &sealed).is_err());
        assert!(sealer.open(DEFAULT_TENANT, "demo.other", "row-1", &sealed).is_err());
        let mut tampered = sealed.clone();
        tampered.pop();
        tampered.push('A');
        assert!(sealer.open(DEFAULT_TENANT, "demo.secret", "row-1", &tampered).is_err());
        assert!(!sealer.needs_reseal(&sealed));
        // Chữ thường cũ: đọc được, cần mã hoá.
        assert_eq!(sealer.open(DEFAULT_TENANT, "p", "r", "plain").unwrap().as_slice(), b"plain");
        assert!(sealer.needs_reseal("plain"));
    }

    #[test]
    fn khoa_cu_mo_duoc_khoa_la_bao_ro() {
        let old = Sealer::new(key(1), Vec::new()).unwrap();
        let sealed = old.seal(DEFAULT_TENANT, "p", "r", b"x").unwrap();
        let rotated = Sealer::new(key(2), vec![key(1)]).unwrap();
        assert_eq!(rotated.open(DEFAULT_TENANT, "p", "r", &sealed).unwrap().as_slice(), b"x");
        assert!(rotated.needs_reseal(&sealed));
        let resealed = rotated.seal(DEFAULT_TENANT, "p", "r", b"x").unwrap();
        assert!(!rotated.needs_reseal(&resealed));
        let forgotten = Sealer::new(key(3), Vec::new()).unwrap();
        let error = forgotten.open(DEFAULT_TENANT, "p", "r", &sealed).unwrap_err();
        assert!(error.to_string().contains("AUTOXIDE_SECRET_KEY_PREVIOUS"), "{error}");
        assert!(format!("{rotated:?}").contains(rotated.current_key_id()));
    }
}
