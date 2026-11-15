//! Khoá ký dùng chung của kernel (D-113): một khoá gốc, dẫn ra khoá riêng cho từng mục đích.
//!
//! - Đặt `AUTOXIDE_SECRET_KEY` thì khoá gốc là giá trị đó.
//! - Không đặt thì kernel **sinh một lần và lưu trong database** (bảng `kernel_secrets` của
//!   module nội bộ `kernel`): khởi động lại hay chạy nhiều máy chủ chung một database vẫn
//!   dùng cùng một khoá, người cài không phải cấu hình gì. Khoá đi theo `autoxide backup`.
//! - Module không dùng thẳng khoá gốc: [`SecretKey::derive`] với một nhãn mục đích (vd
//!   `autoxide/antispam/form-token/v1`) cho ra một khoá 32 byte riêng — lộ khoá của một mục
//!   đích không lộ khoá khác.
//! - Không thứ gì sống lâu được phụ thuộc khoá này (đổi khoá là mọi token ký bằng nó hết
//!   dùng được): link huỷ đăng ký trong email dùng token ngẫu nhiên lưu bản băm.
//!
//! - Secret module cần đọc lại (secret TOTP, secret ký webhook…) được mã hoá khi lưu bằng
//!   [`Sealer`] ([`SecretKey::sealer`], D-257). Xoay khoá: khoá mới vào `AUTOXIDE_SECRET_KEY`, khoá
//!   cũ vào `AUTOXIDE_SECRET_KEY_PREVIOUS`, chạy `autoxide secrets reseal`, rồi bỏ khoá cũ.
//!
//! Module lấy khoá lúc khởi động: `let key = context.require::<SecretKey>()?;`.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, OnConflict, Query};
use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

use crate::config::{ConfigReader, Invalid, Setting};
use crate::database::Database;
use crate::error::Error;

pub use super::seal::{Sealer, is_sealed};

/// Độ dài tối thiểu của `AUTOXIDE_SECRET_KEY`.
const MIN_LENGTH: usize = 32;

/// Tên dòng giữ khoá gốc trong `kernel_secrets`.
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

/// Đọc `AUTOXIDE_SECRET_KEY_PREVIOUS` (không bắt buộc): danh sách, cách nhau dấu phẩy.
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

/// Đọc `AUTOXIDE_SECRET_KEY` (không bắt buộc).
pub(crate) fn read_setting(reader: &mut ConfigReader<'_>) -> Result<Option<Vec<u8>>, Invalid> {
    reader.optional_with(&SECRET_KEY, |raw| {
        if raw.len() < MIN_LENGTH {
            Err(format!(
                "must be at least {MIN_LENGTH} characters long; generate one with `openssl rand -base64 48`"
            ))
        } else {
            Ok(raw.as_bytes().to_vec())
        }
    })
}

/// Khoá ký dùng chung. Clone rẻ: mọi bản clone dùng chung một khoá gốc (đọc database nhiều
/// nhất một lần).
#[derive(Clone)]
pub struct SecretKey {
    database: Database,
    configured: Option<Arc<Vec<u8>>>,
    /// Khoá gốc cũ (`AUTOXIDE_SECRET_KEY_PREVIOUS`): chỉ để mở secret đã mã hoá bằng chúng.
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
    /// Khoá của chương trình: `configured` (giá trị `AUTOXIDE_SECRET_KEY`), hoặc khoá lưu trong
    /// database. Database phải đã chạy migration của module `kernel`.
    pub fn new(database: Database, configured: Option<Vec<u8>>) -> Self {
        SecretKey {
            database,
            configured: configured.map(Arc::new),
            previous: Arc::new(Vec::new()),
            sealer: Arc::new(OnceCell::new()),
            master: Arc::new(OnceCell::new()),
        }
    }

    /// Thêm khoá gốc cũ (`AUTOXIDE_SECRET_KEY_PREVIOUS`) để mở secret đã mã hoá bằng chúng.
    pub fn with_previous(mut self, previous: Vec<Vec<u8>>) -> Self {
        self.previous = Arc::new(previous);
        self
    }

    /// Bộ mã hoá secret: khoá hiện tại dẫn từ khoá gốc; khoá cũ dẫn từ
    /// `AUTOXIDE_SECRET_KEY_PREVIOUS` và — khi `AUTOXIDE_SECRET_KEY` đã đặt — từ khoá gốc kernel đã
    /// sinh trong database trước đó, để lần đầu đặt biến môi trường không làm mất secret nào.
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

    /// `true` khi khoá gốc đến từ `AUTOXIDE_SECRET_KEY`; `false` khi kernel tự sinh và giữ nó
    /// trong database (ai có bản sao database cũng có khoá).
    pub fn is_configured(&self) -> bool {
        self.configured.is_some()
    }

    /// Khoá 32 byte riêng cho mục đích `purpose`: HMAC-SHA256 của nhãn trên khoá gốc. Cùng
    /// khoá gốc và cùng nhãn thì luôn ra cùng khoá.
    pub async fn derive(&self, purpose: &str) -> Result<[u8; 32], Error> {
        derive_from(self.master().await?, purpose)
    }

    async fn master(&self) -> Result<&[u8], Error> {
        if let Some(configured) = &self.configured {
            return Ok(configured.as_slice());
        }
        let master = self.master.get_or_try_init(|| stored_key(&self.database)).await?;
        Ok(master.as_slice())
    }
}

/// Cấu hình khoá đã đọc (`AUTOXIDE_SECRET_KEY`, `AUTOXIDE_SECRET_KEY_PREVIOUS`), dùng chung giữa
/// module `kernel` (đọc lúc `configure`) và `Migrator` (upgrade dữ liệu cần đúng khoá của chương
/// trình). Clone rẻ.
#[derive(Clone, Default)]
pub(crate) struct KeyConfig(Arc<std::sync::RwLock<Keys>>);

/// Khoá gốc đã cấu hình (nếu có) và các khoá gốc cũ.
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

    /// `SecretKey` của chương trình trên `database`.
    pub(crate) fn key_for(&self, database: &Database) -> SecretKey {
        let keys = self.0.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        SecretKey::new(database.clone(), keys.configured.clone())
            .with_previous(keys.previous.clone())
    }
}

/// HMAC-SHA256 của nhãn `purpose` trên khoá gốc `master`.
fn derive_from(master: &[u8], purpose: &str) -> Result<[u8; 32], Error> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(master)
        .map_err(|error| Error::internal(format!("hmac key: {error}")))?;
    mac.update(purpose.as_bytes());
    Ok(mac.finalize().into_bytes().into())
}

/// Khoá gốc lưu trong database; chưa có thì sinh rồi lưu. Hai máy chủ cùng sinh lúc khởi động
/// lần đầu: `ON CONFLICT DO NOTHING` giữ khoá của bên ghi trước, cả hai đọc lại cùng một khoá.
async fn stored_key(database: &Database) -> Result<Vec<u8>, Error> {
    if let Some(found) = read_key(database.write()).await? {
        return Ok(found);
    }
    let fresh = random_token(48)?;
    let insert = Query::insert()
        .into_table(Alias::new("kernel_secrets"))
        .columns([Alias::new("name"), Alias::new("value"), Alias::new("created_at")])
        .values_panic([SIGNING_KEY.into(), fresh.into(), Utc::now().into()])
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

/// `bytes` byte ngẫu nhiên từ bộ sinh số ngẫu nhiên của hệ điều hành, mã hoá base64url.
pub fn random_token(bytes: usize) -> Result<String, Error> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer)
        .map_err(|error| Error::internal(format!("random number generator: {error}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

/// SHA-256 của `text`, mã hoá base64url (43 ký tự). Để lưu token ngẫu nhiên: database chỉ giữ
/// bản băm, lộ bảng cũng không dùng được token. Không cần Argon2 — token ngẫu nhiên 256 bit
/// không đoán thử được như mật khẩu.
pub fn sha256(text: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(text.as_bytes()))
}
