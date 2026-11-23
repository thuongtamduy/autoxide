//! Encrypts secrets at rest (step 33, D-257): secrets a module must be able to read back (TOTP secrets, webhook
//! signing secrets, each tenant's partner connection keys) are stored as
//! `sealed:v1:<key_id>:<base64url(nonce || ciphertext || tag)>`.
//!
//! - Algorithm: AES-256-GCM (AEAD) from `ring`; a random 96-bit nonce for each encryption.
//! - Key: `SecretKey::derive("autoxide/kernel/seal/v1")` of the root key — never stored next to the data when
//!   `AUTOXIDE_SECRET_KEY` is set. `key_id` = the first 8 hex characters of the key's SHA-256: it tells which
//!   key encrypted a value without revealing the key.
//! - Associated data (AAD) = purpose + tenant + row identifier: copying an encrypted value to another
//!   row, another tenant or another purpose cannot be opened.
//! - Key rotation: old keys can still open values; [`Sealer::needs_reseal`] tells which values need
//!   re-encrypting with the current key (`autoxide secrets reseal`).
//! - A value without the `sealed:` prefix is plaintext from an older version: [`Sealer::open`] returns it
//!   verbatim (only during the transition) and [`Sealer::needs_reseal`] is `true`.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::Error;
use crate::tenancy::TenantId;

/// Label for deriving the encryption key from the root key.
pub(crate) const PURPOSE: &str = "autoxide/kernel/seal/v1";

const PREFIX: &str = "sealed:v1:";

/// Secret encryptor: the current key and old keys (by `key_id`). Cheap to clone.
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
    /// An encryptor with the current key `current` and old keys `previous` (each key 32 bytes).
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

    /// `key_id` of the current key.
    pub fn current_key_id(&self) -> &str {
        self.keys.first().map_or("", |(id, _)| id.as_str())
    }

    /// Encrypts `plaintext` of tenant `tenant`, for purpose `purpose` (e.g. `webhooks.endpoint_secret`),
    /// bound to row `row` (e.g. the row's ID).
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

    /// Opens a value `sealed` encrypted with [`seal`](Self::seal) with the same tenant, purpose and row.
    /// A plaintext value from an older version (no `sealed:` prefix) is returned verbatim.
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

    /// A value that needs re-encrypting with the current key: old plaintext, or encrypted with an old key.
    pub fn needs_reseal(&self, sealed: &str) -> bool {
        let current = format!("{PREFIX}{}:", self.current_key_id());
        !sealed.starts_with(&current)
    }
}

/// A value that is already encrypted (has the `sealed:` prefix), whichever key was used.
pub fn is_sealed(value: &str) -> bool {
    value.starts_with("sealed:")
}

fn aad(tenant: TenantId, purpose: &str, row: &str) -> Vec<u8> {
    // Byte 0 ngăn cách: không ghép hai bộ (mục đích, dòng) khác nhau thành cùng một chuỗi.
    format!("{purpose}\0{tenant}\0{row}").into_bytes()
}

/// The first 8 hex characters of the key's SHA-256.
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
        // Đổi một ký tự ở giữa phần mã hoá (luôn khác ký tự cũ).
        let mut tampered: Vec<char> = sealed.chars().collect();
        let middle = tampered.len() - 10;
        tampered[middle] = if tampered[middle] == 'A' { 'B' } else { 'A' };
        let tampered: String = tampered.into_iter().collect();
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
