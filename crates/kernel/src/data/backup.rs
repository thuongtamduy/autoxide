//! Lõi của bản backup (D-073, bước 39): lệnh `autoxide backup` và job backup theo lịch dùng CHUNG
//! đường này — một việc, một cách làm.
//!
//! Bản backup là một cây file, đọc được trên mọi máy và cho mọi cách cài:
//!
//! ```text
//! backup.json       mô tả: phiên bản, lúc tạo, số dòng mỗi bảng, số file
//! database.sqlite   dữ liệu mọi module, chép từ SQLite hay PostgreSQL
//! files/<khoá>      file từ nơi lưu (ổ đĩa hay S3), khi có chép file
//! ```
//!
//! Nơi ghi là một [`BackupSink`]: thư mục trên máy ([`DirectorySink`], lệnh CLI) hay nơi lưu file
//! dưới một tiền tố ([`StorageSink`], job theo lịch). Database được đọc trong một snapshot
//! (`copy_modules`) nên nhất quán dù máy chủ đang chạy, và KHÔNG giữ transaction ghi nào trên nguồn.
//! File dưới [`BACKUP_PREFIX`] (chính các bản backup theo lịch) không bao giờ được chép: không thì
//! bản sau chứa bản trước, phình theo cấp số.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt as _;

use crate::config::{ConfigReader, ConfigSource, DatabaseConfig};
use crate::database::Database;
use crate::error::Error;
use crate::migration::Migrator;
use crate::storage::{ByteStream, Storage};
use crate::transfer::copy_modules;

/// Tên file mô tả.
pub const MANIFEST: &str = "backup.json";
/// Tên file dữ liệu.
pub const DATABASE_FILE: &str = "database.sqlite";
/// Thư mục con chứa file.
pub const FILES_DIR: &str = "files";
/// Phiên bản của định dạng bản backup.
pub const FORMAT: u64 = 1;
/// Tiền tố khoá nơi lưu của bản backup theo lịch — bị loại khỏi phần chép file của mọi bản backup.
pub const BACKUP_PREFIX: &str = "kernel/backups/";

/// Mô tả một bản backup (`backup.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackupManifest {
    pub format: u64,
    /// Phiên bản Autoxide đã tạo bản backup.
    pub autoxide: String,
    pub created_at: DateTime<Utc>,
    /// Loại database nguồn: `sqlite` hay `postgres`.
    pub source: String,
    pub tables: Vec<BackupTable>,
    pub files: u64,
    pub file_bytes: u64,
}

impl BackupManifest {
    /// Tổng số dòng mọi bảng.
    pub fn rows(&self) -> u64 {
        self.tables.iter().map(|table| table.rows).sum()
    }
}

/// Số dòng đã chép của một bảng.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackupTable {
    pub table: String,
    pub rows: u64,
}

/// Nơi ghi một bản backup. `relative` là đường dẫn trong bản backup, phân cách bằng `/`.
#[async_trait::async_trait]
pub trait BackupSink: Send + Sync {
    /// Ghi `content` vào `relative`; trả số byte đã ghi.
    async fn put(&self, relative: &str, content: ByteStream) -> Result<u64, Error>;
}

/// Ghi vào một thư mục trên máy (lệnh `autoxide backup`).
#[derive(Debug, Clone)]
pub struct DirectorySink {
    root: PathBuf,
}

impl DirectorySink {
    /// Thư mục `root` (đã có, quyền `0700` — xem [`private_dir`]).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        DirectorySink { root: root.into() }
    }
}

#[async_trait::async_trait]
impl BackupSink for DirectorySink {
    async fn put(&self, relative: &str, mut content: ByteStream) -> Result<u64, Error> {
        use tokio::io::AsyncWriteExt as _;
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io(&path))?;
        }
        let mut file = tokio::fs::File::create(&path).await.map_err(io(&path))?;
        let mut bytes = 0u64;
        while let Some(chunk) = content.next().await {
            let chunk = chunk?;
            bytes += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(io(&path))?;
        }
        file.sync_all().await.map_err(io(&path))?;
        Ok(bytes)
    }
}

/// Ghi vào nơi lưu file dưới tiền tố `prefix` (kết thúc bằng `/`), cho job theo lịch.
#[derive(Clone)]
pub struct StorageSink {
    storage: Arc<dyn Storage>,
    prefix: String,
}

impl StorageSink {
    pub fn new(storage: Arc<dyn Storage>, prefix: impl Into<String>) -> Self {
        StorageSink { storage, prefix: prefix.into() }
    }
}

impl std::fmt::Debug for StorageSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageSink").field("prefix", &self.prefix).finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl BackupSink for StorageSink {
    async fn put(&self, relative: &str, content: ByteStream) -> Result<u64, Error> {
        self.storage.put(&format!("{}{relative}", self.prefix), content).await
    }
}

/// Tạo một bản backup của `database` vào `sink`: bảng của `modules` (đã áp dụng đủ `migrator`), và
/// mọi file của `files` (nếu có) trừ [`BACKUP_PREFIX`]. `scratch` là thư mục tạm riêng tư để dựng
/// file SQLite trước khi đẩy vào `sink` (cần chỗ trống cỡ database); hàm tự dọn nó, kể cả khi lỗi.
pub async fn write_backup(
    database: &Database,
    migrator: &Migrator,
    modules: &[&'static str],
    files: Option<Arc<dyn Storage>>,
    sink: &dyn BackupSink,
    scratch: &Path,
) -> Result<BackupManifest, Error> {
    private_dir(scratch).map_err(io(scratch))?;
    let result = async {
        let path = scratch.join(DATABASE_FILE);
        let copy = open_sqlite(&path, migrator).await?;
        let tables = copy_modules(database, &copy, modules).await;
        // Đóng trước khi đọc file: mọi trang đã nằm trong file chính, không còn file `-wal`.
        copy.close().await?;
        let tables = tables.map_err(|error| Error::internal(error.to_string()))?;
        sink.put(DATABASE_FILE, read_file(path).await?).await?;
        let (count, bytes) = match files {
            Some(storage) => copy_files(storage.as_ref(), sink).await?,
            None => (0, 0),
        };
        let manifest = BackupManifest {
            format: FORMAT,
            autoxide: env!("CARGO_PKG_VERSION").to_owned(),
            created_at: Utc::now(),
            source: format!("{:?}", database.backend()).to_lowercase(),
            tables: tables
                .iter()
                .map(|table| BackupTable { table: table.table.clone(), rows: table.rows })
                .collect(),
            files: count,
            file_bytes: bytes,
        };
        let text = serde_json::to_vec_pretty(&manifest).map_err(Error::internal)?;
        let stream = futures_util::stream::once(async move { Ok(bytes::Bytes::from(text)) });
        sink.put(MANIFEST, stream.boxed()).await?;
        Ok(manifest)
    }
    .await;
    let _ = tokio::fs::remove_dir_all(scratch).await;
    result
}

/// Chép mọi file của `storage` (trừ [`BACKUP_PREFIX`]) vào `files/` của `sink`.
async fn copy_files(storage: &dyn Storage, sink: &dyn BackupSink) -> Result<(u64, u64), Error> {
    let mut files = 0;
    let mut bytes = 0;
    for key in storage.keys().await? {
        if key.starts_with(BACKUP_PREFIX) {
            continue;
        }
        // File bị xoá trong lúc backup: bỏ qua.
        let Some(stored) = storage.get(&key).await? else { continue };
        bytes += sink.put(&format!("{FILES_DIR}/{key}"), stored.content).await?;
        files += 1;
    }
    Ok((files, bytes))
}

/// File SQLite ở `path` (tạo khi chưa có), đã áp dụng mọi migration của `migrator`.
pub async fn open_sqlite(path: &Path, migrator: &Migrator) -> Result<Database, Error> {
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let source = ConfigSource::from_pairs([("DATABASE_URL", url.as_str())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let (config, _) =
        reader.finish(config).map_err(|errors| Error::internal(errors.to_string()))?;
    let database =
        Database::connect(&config).await.map_err(|error| Error::internal(error.to_string()))?;
    if let Err(error) = migrator.up(&database).await {
        let _ = database.close().await;
        return Err(Error::internal(error.to_string()));
    }
    Ok(database)
}

/// Tạo thư mục `path` (và các thư mục cha) rồi, trên Unix, chỉ để chủ của nó đọc / ghi / mở
/// (`0700`). Backup chứa hash mật khẩu, secret webhook, MFA, và khoá ký khi nó nằm trong
/// database: người dùng khác trên cùng máy không được đọc (D-158). Khoá thư mục là đủ — file bên
/// trong không mở được qua thư mục đã khoá.
pub fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Nội dung file `path` dưới dạng luồng, đọc từng đoạn 64 KiB.
pub async fn read_file(path: PathBuf) -> Result<ByteStream, Error> {
    let file = tokio::fs::File::open(&path).await.map_err(io(&path))?;
    let stream = futures_util::stream::unfold(Some(file), |file| async move {
        let mut file = file?;
        let mut buffer = vec![0u8; 64 * 1024];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(read) => {
                buffer.truncate(read);
                Some((Ok(bytes::Bytes::from(buffer)), Some(file)))
            }
            Err(error) => Some((Err(Error::internal(error.to_string())), None)),
        }
    });
    Ok(stream.boxed())
}

/// Lỗi vào ra kèm đường dẫn.
fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |error| Error::internal(format!("{}: {error}", path.display()))
}
