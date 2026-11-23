//! Core of the backup (D-073, step 39): the `autoxide backup` command and the scheduled backup job share THIS
//! path — one task, one way to do it.
//!
//! A backup is a file tree, readable on any machine and for every installation:
//!
//! ```text
//! backup.json       description: version, creation time, row count per table, file count
//! database.sqlite   data of every module, copied from SQLite or PostgreSQL
//! files/<key>       files from the storage (disk or S3), when files are copied
//! ```
//!
//! The destination is a [`BackupSink`]: a directory on the machine ([`DirectorySink`], CLI command) or a file
//! store under a prefix ([`StorageSink`], scheduled job). The database is read in one snapshot
//! (`copy_modules`), so it is consistent while the server keeps running, and NO write transaction is held on
//! the source. Files under [`BACKUP_PREFIX`] (the scheduled backups themselves) are never copied: otherwise
//! each backup would contain the previous one and grow exponentially.

pub mod scheduled;

use crate::time::Timestamp;
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::AsyncReadExt as _;

use crate::config::{ConfigReader, ConfigSource, DatabaseConfig};
use crate::database::Database;
use crate::error::Error;
use crate::migration::Migrator;
use crate::storage::{ByteStream, Storage};
use crate::transfer::copy_modules;

/// Name of the description file.
pub const MANIFEST: &str = "backup.json";
/// Name of the data file.
pub const DATABASE_FILE: &str = "database.sqlite";
/// Subdirectory that holds the files.
pub const FILES_DIR: &str = "files";
/// Version of the backup format.
pub const FORMAT: u64 = 1;
/// Storage key prefix of scheduled backups; excluded from the file copy of every backup.
pub const BACKUP_PREFIX: &str = "kernel/backups/";

/// Describes a backup (`backup.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackupManifest {
    pub format: u64,
    /// Autoxide version that created the backup.
    pub autoxide: String,
    pub created_at: Timestamp,
    /// Kind of source database: `sqlite` or `postgres`.
    pub source: String,
    pub tables: Vec<BackupTable>,
    pub files: u64,
    pub file_bytes: u64,
}

impl BackupManifest {
    /// Total row count across all tables.
    pub fn rows(&self) -> u64 {
        self.tables.iter().map(|table| table.rows).sum()
    }
}

/// Number of rows copied for a table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BackupTable {
    pub table: String,
    pub rows: u64,
}

/// Destination of a backup. `relative` is the path inside the backup, separated by `/`.
#[async_trait::async_trait]
pub trait BackupSink: Send + Sync {
    /// Writes `content` to `relative`; returns the number of bytes written.
    async fn put(&self, relative: &str, content: ByteStream) -> Result<u64, Error>;
}

/// Writes to a directory on the machine (the `autoxide backup` command).
#[derive(Debug, Clone)]
pub struct DirectorySink {
    root: PathBuf,
}

impl DirectorySink {
    /// The `root` directory (already exists, mode `0700`; see [`private_dir`]).
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

/// Writes to the file store under the prefix `prefix` (ending with `/`), for the scheduled job.
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

/// Creates a backup of `database` in `sink`: the tables of `modules` (with `migrator` fully applied) and
/// every file of `files` (if any) except [`BACKUP_PREFIX`]. `scratch` is a private temporary directory used to
/// build the SQLite file before pushing it to `sink` (needs free space about the size of the database); the
/// function cleans it up itself, even on error.
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
            created_at: Timestamp::now(),
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

/// Copies every file of `storage` (except [`BACKUP_PREFIX`]) into `files/` of `sink`.
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

/// The SQLite file at `path` (created if missing), with every migration of `migrator` applied.
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

/// Creates the directory `path` (and its parents) and, on Unix, lets only its owner read / write / open it
/// (`0700`). A backup contains password hashes, webhook secrets, MFA and, when stored in the database, signing
/// keys: other users on the same machine must not read it (D-158). Locking the directory is enough: files
/// inside cannot be opened through a locked directory.
pub fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Content of the file `path` as a stream, read in 64 KiB chunks.
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

/// I/O error with the path attached.
fn io(path: &Path) -> impl Fn(std::io::Error) -> Error + '_ {
    move |error| Error::internal(format!("{}: {error}", path.display()))
}
