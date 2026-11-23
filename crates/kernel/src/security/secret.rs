//! The kernel's shared signing key (D-113): one master key, from which a separate key is derived for each
//! purpose.
//!
//! - If `AUTOXIDE_SECRET_KEY` is set, the master key is that value.
//! - If not set, the kernel **generates one once and stores it in the database** (table `kernel_secrets` of the
//!   internal module `kernel`): after a restart, or when several servers share one database, the same key is
//!   still used, and the installer has nothing to configure. The key travels with `autoxide backup`.
//! - Modules do not use the master key directly: [`SecretKey::derive`] with a purpose label (e.g.
//!   `autoxide/antispam/form-token/v1`) yields a separate 32-byte key; leaking the key of one purpose does not
//!   leak another.
//! - Nothing long-lived may depend on this key (changing the key makes every token signed with it unusable):
//!   unsubscribe links in emails use a random token whose hash is stored.
//!
//! - Secrets a module needs to read back (TOTP secrets, webhook signing secrets, ...) are encrypted at rest
//!   with a [`Sealer`] ([`SecretKey::sealer`], D-257). Key rotation: put the new key in
//!   `AUTOXIDE_SECRET_KEY`, the old key in `AUTOXIDE_SECRET_KEY_PREVIOUS`, run `autoxide secrets reseal`,
//!   then drop the old key.
//!
//! A module gets the key at startup: `let key = context.require::<SecretKey>()?;`.

use crate::time::Timestamp;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Query};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

use crate::config::{ConfigReader, Invalid, Setting};
use crate::database::Database;
use crate::error::Error;

pub use super::seal::{Sealer, is_sealed};

/// Minimum length of `AUTOXIDE_SECRET_KEY`.
const MIN_LENGTH: usize = 32;

/// Derivation label of [`SecretKey::phone_key`] / [`SecretKey::email_key`].
const CONTACT_KEY_PURPOSE: &str = "autoxide/contact-key/v1";

/// Name of the row that holds the master key in `kernel_secrets`.
const SIGNING_KEY: &str = "signing_key";

pub(crate) const SECRET_KEY: Setting = Setting::new(
    "AUTOXIDE_SECRET_KEY",
    "Key that signs short-lived tokens of every module (form tokens, anti-spam challenges): at \
     least 32 characters, random. Generate one with `openssl rand -base64 48`. When unset, one \
     is generated once and kept in the database, so every server sharing the database uses it. \
     Changing it invalidates tokens already handed out (they live minutes, not days).",
)
.secret()
.with_example("change-me-to-a-long-random-value-from-openssl-rand");

pub(crate) const SECRET_KEY_PREVIOUS: Setting = Setting::new(
    "AUTOXIDE_SECRET_KEY_PREVIOUS",
    "Old values of AUTOXIDE_SECRET_KEY, separated by commas, while rotating the key: secrets stored \
     encrypted with an old key stay readable until `autoxide secrets reseal` encrypts them again \
     with the current one. Remove them after that.",
)
.secret();

/// Reads `AUTOXIDE_SECRET_KEY_PREVIOUS` (optional): a comma-separated list.
pub(crate) fn read_previous(reader: &mut ConfigReader<'_>) -> Result<Vec<Vec<u8>>, Invalid> {
    let raw = reader.optional_with(&SECRET_KEY_PREVIOUS, |raw| {
        let keys: Vec<Vec<u8>> = raw
            .split(',')
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(|key| key.as_bytes().to_vec())
            .collect();
        if keys.iter().any(|key| key.len() < MIN_LENGTH) {
            Err(format!("every key must be at least {MIN_LENGTH} characters long"))
        } else {
            Ok(keys)
        }
    })?;
    Ok(raw.unwrap_or_default())
}

/// Sample value of `.env.example` (and any value that starts like it): copying the sample as is means no real
/// key is set, since anyone reading the repo knows it.
const PLACEHOLDER_PREFIX: &str = "change-me";

/// Reads `AUTOXIDE_SECRET_KEY` (optional).
pub(crate) fn read_setting(reader: &mut ConfigReader<'_>) -> Result<Option<Vec<u8>>, Invalid> {
    reader.optional_with(&SECRET_KEY, |raw| {
        if raw.trim().to_ascii_lowercase().starts_with(PLACEHOLDER_PREFIX) {
            Err("is still the example value; generate a key with `openssl rand -base64 48`"
                .to_owned())
        } else if raw.len() < MIN_LENGTH {
            Err(format!(
                "must be at least {MIN_LENGTH} characters long; generate one with `openssl rand -base64 48`"
            ))
        } else {
            Ok(raw.as_bytes().to_vec())
        }
    })
}

/// Shared signing key. Cheap to clone: every clone shares one master key (the database is read at most
/// once).
#[derive(Clone)]
pub struct SecretKey {
    database: Database,
    configured: Option<Arc<Vec<u8>>>,
    /// Old master keys (`AUTOXIDE_SECRET_KEY_PREVIOUS`): only to open secrets encrypted with them.
    previous: Arc<Vec<Vec<u8>>>,
    sealer: Arc<OnceCell<Sealer>>,
    // `OnceCell` của tokio: lần gọi đầu đọc (hoặc sinh) khoá gốc, các lần sau và các lời gọi
    // chạy cùng lúc dùng lại kết quả đó.
    master: Arc<OnceCell<Vec<u8>>>,
}

impl std::fmt::Debug for SecretKey {
    // Không bao giờ in khoá.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretKey")
            .field("configured", &self.configured.is_some())
            .finish_non_exhaustive()
    }
}

impl SecretKey {
    /// The program's key: `configured` (the value of `AUTOXIDE_SECRET_KEY`), or the key stored in the
    /// database. The database must already have run the `kernel` module's migrations.
    pub fn new(database: Database, configured: Option<Vec<u8>>) -> Self {
        SecretKey {
            database,
            configured: configured.map(Arc::new),
            previous: Arc::new(Vec::new()),
            sealer: Arc::new(OnceCell::new()),
            master: Arc::new(OnceCell::new()),
        }
    }

    /// Adds old master keys (`AUTOXIDE_SECRET_KEY_PREVIOUS`) to open secrets encrypted with them.
    pub fn with_previous(mut self, previous: Vec<Vec<u8>>) -> Self {
        self.previous = Arc::new(previous);
        self
    }

    /// Secret encryptor: the current key is derived from the master key; old keys are derived from
    /// `AUTOXIDE_SECRET_KEY_PREVIOUS` and, when `AUTOXIDE_SECRET_KEY` is set, from the master key the kernel
    /// generated earlier in the database, so setting the environment variable for the first time does not
    /// lose any secret.
    pub async fn sealer(&self) -> Result<Sealer, Error> {
        let sealer = self
            .sealer
            .get_or_try_init(|| async {
                let current = self.derive(super::seal::PURPOSE).await?;
                let mut previous: Vec<[u8; 32]> = Vec::new();
                for old in self.previous.iter() {
                    previous.push(derive_from(old, super::seal::PURPOSE)?);
                }
                if self.configured.is_some()
                    && let Some(stored) = read_key(self.database.read()).await?
                {
                    previous.push(derive_from(&stored, super::seal::PURPOSE)?);
                }
                Sealer::new(current, previous)
            })
            .await?;
        Ok(sealer.clone())
    }

    /// `true` when the master key comes from `AUTOXIDE_SECRET_KEY`; `false` when the kernel generated it
    /// itself and keeps it in the database (anyone with a copy of the database has the key).
    pub fn is_configured(&self) -> bool {
        self.configured.is_some()
    }

    /// A separate 32-byte key for purpose `purpose`: the HMAC-SHA256 of the label under the master key. The
    /// same master key and the same label always give the same key.
    pub async fn derive(&self, purpose: &str) -> Result<[u8; 32], Error> {
        derive_from(self.master().await?, purpose)
    }

    /// A separate 32-byte key for purpose `purpose` **of tenant `tenant`** (step 33): a token signed with this
    /// tenant's key and carried to another tenant cannot be opened. The default tenant uses exactly the key of
    /// [`derive`](Self::derive), so a single-tenant installation changes no key on upgrade.
    pub async fn derive_for(
        &self,
        tenant: crate::tenancy::TenantId,
        purpose: &str,
    ) -> Result<[u8; 32], Error> {
        if tenant == crate::tenancy::DEFAULT_TENANT {
            return self.derive(purpose).await;
        }
        // Byte 0 ngăn cách: không ghép (mục đích, khách) khác nhau thành cùng một nhãn.
        derive_from(self.master().await?, &format!("{purpose}\0tenant\0{tenant}"))
    }

    /// Contact key of phone number `phone` within tenant `tenant` (D-194, spec 45 "anti fake orders"): the
    /// HMAC-SHA256 of the E.164 number under the tenant's own key, encoded as base64url (43 characters). The
    /// same number in the same tenant always gives the same key (grouping orders, delivery statistics); two
    /// tenants give different keys, so data cannot be compared across tenants. Modules store this key instead
    /// of the number for lookups, and never show it to users.
    pub async fn phone_key(
        &self,
        tenant: crate::tenancy::TenantId,
        phone: &crate::sms::PhoneNumber,
    ) -> Result<String, Error> {
        self.contact_key(tenant, "phone", phone.as_str()).await
    }

    /// Like [`phone_key`](Self::phone_key) for email: compared case-insensitively, with leading and trailing
    /// whitespace stripped.
    pub async fn email_key(
        &self,
        tenant: crate::tenancy::TenantId,
        email: &str,
    ) -> Result<String, Error> {
        self.contact_key(tenant, "email", &email.trim().to_lowercase()).await
    }

    async fn contact_key(
        &self,
        tenant: crate::tenancy::TenantId,
        kind: &str,
        normalized: &str,
    ) -> Result<String, Error> {
        let key = self.derive_for(tenant, CONTACT_KEY_PURPOSE).await?;
        let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(&key)
            .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
        // Byte 0 ngăn cách loại và giá trị: số điện thoại không trùng khoá với email.
        mac.update(kind.as_bytes());
        mac.update(b"\0");
        mac.update(normalized.as_bytes());
        Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
    }

    async fn master(&self) -> Result<&[u8], Error> {
        if let Some(configured) = &self.configured {
            return Ok(configured.as_slice());
        }
        let master = self.master.get_or_try_init(|| stored_key(&self.database)).await?;
        Ok(master.as_slice())
    }
}

/// Key configuration as read (`AUTOXIDE_SECRET_KEY`, `AUTOXIDE_SECRET_KEY_PREVIOUS`), shared between the
/// `kernel` module (reads it at `configure`) and `Migrator` (data upgrades need the program's exact key).
/// Cheap to clone.
#[derive(Clone, Default)]
pub(crate) struct KeyConfig(Arc<std::sync::RwLock<Keys>>);

/// The configured master key (if any) and the old master keys.
#[derive(Default)]
struct Keys {
    configured: Option<Vec<u8>>,
    previous: Vec<Vec<u8>>,
}

impl std::fmt::Debug for KeyConfig {
    // Không bao giờ in khoá.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyConfig")
    }
}

impl KeyConfig {
    pub(crate) fn set(&self, configured: Option<Vec<u8>>, previous: Vec<Vec<u8>>) {
        // Khoá hỏng (một luồng panic khi đang giữ) vẫn ghi được dữ liệu bên trong.
        let mut keys = self.0.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        *keys = Keys { configured, previous };
    }

    /// Whether `AUTOXIDE_SECRET_KEY` is set (if not, the master key is generated and stored in the database).
    pub(crate) fn is_configured(&self) -> bool {
        let keys = self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        keys.configured.is_some()
    }

    /// The program's `SecretKey` on `database`.
    pub(crate) fn key_for(&self, database: &Database) -> SecretKey {
        let keys = self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        SecretKey::new(database.clone(), keys.configured.clone())
            .with_previous(keys.previous.clone())
    }
}

/// HMAC-SHA256 of label `purpose` under the master key `master`.
fn derive_from(master: &[u8], purpose: &str) -> Result<[u8; 32], Error> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(master)
        .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
    mac.update(purpose.as_bytes());
    Ok(mac.finalize().into_bytes().into())
}

/// The master key stored in the database; if absent, generates it and stores it. Two servers generating at the
/// first startup at the same time: `ON CONFLICT DO NOTHING` keeps the earlier writer's key, and both read back
/// the same key.
async fn stored_key(database: &Database) -> Result<Vec<u8>, Error> {
    if let Some(found) = read_key(database.write()).await? {
        return Ok(found);
    }
    let fresh = random_token(48)?;
    let insert = Query::insert()
        .into_table(Alias::new("kernel_secrets"))
        .columns([Alias::new("name"), Alias::new("value"), Alias::new("created_at")])
        .values_panic([SIGNING_KEY.into(), fresh.into(), Timestamp::now().into()])
        .on_conflict(OnConflict::column(Alias::new("name")).do_nothing().to_owned())
        .to_owned();
    let connection = database.write();
    connection.execute(&insert).await?;
    let stored = read_key(connection).await?;
    let stored = stored.ok_or_else(|| Error::internal("the signing key could not be stored"))?;
    tracing::info!(
        "generated the signing key and stored it in the database (AUTOXIDE_SECRET_KEY is not set)"
    );
    Ok(stored)
}

async fn read_key<C: ConnectionTrait>(connection: &C) -> Result<Option<Vec<u8>>, Error> {
    let select = Query::select()
        .column(Alias::new("value"))
        .from(Alias::new("kernel_secrets"))
        .and_where(Expr::col(Alias::new("name")).eq(SIGNING_KEY))
        .to_owned();
    let row = connection.query_one(&select).await?;
    match row {
        Some(row) => Ok(Some(row.try_get::<String>("", "value")?.into_bytes())),
        None => Ok(None),
    }
}

/// `bytes` random bytes from the operating system's random number generator, encoded as base64url.
pub fn random_token(bytes: usize) -> Result<String, Error> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer)
        .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// SHA-256 of `text`, encoded as base64url (43 characters). For storing random tokens: the database keeps only
/// the hash, so a leaked table cannot use the tokens. Argon2 is not needed: a 256-bit random token cannot be
/// guessed the way a password can.
pub fn sha256(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(value: &str) -> Result<Option<Vec<u8>>, String> {
        let source = ConfigSource::from_pairs([("AUTOXIDE_SECRET_KEY", value)]);
        let mut reader = ConfigReader::new(&source);
        let key = read_setting(&mut reader);
        reader.finish(key).map(|(key, _)| key).map_err(|errors| errors.to_string())
    }

    #[test]
    fn tu_choi_gia_tri_mau_cua_env_example() {
        let error = read("change-me-to-a-long-random-value-from-openssl-rand").unwrap_err();
        assert!(error.contains("example value"), "{error}");
        assert!(read("Change-Me-please-0123456789abcdef0123456789").is_err());
        assert!(read("too-short").unwrap_err().contains("at least 32"));
        let good = "q8Vt0m3nYc2Lw9Rk5Jx1Hb7Df4Gs6Pa0Ze2Ui8Oy";
        assert_eq!(read(good).unwrap(), Some(good.as_bytes().to_vec()));
    }
}
