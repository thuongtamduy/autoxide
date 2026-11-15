//! Lệnh `autoxide backup` và `autoxide restore` (D-073).
//!
//! Bản backup là một thư mục, đọc được trên mọi máy và cho mọi cách cài:
//!
//! ```text
//! <thư mục>/backup.json       mô tả: phiên bản, lúc tạo, số dòng mỗi bảng, số file
//! <thư mục>/database.sqlite   dữ liệu mọi module, chép từ SQLite hay PostgreSQL
//! <thư mục>/files/<khoá>      mọi file từ nơi lưu (ổ đĩa hay S3)
//! ```
//!
//! Database được đọc trong một snapshot nên nhất quán dù máy chủ đang chạy; file được
//! chép sau database. Restore chỉ nhận một database mới, trống.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::migration::Migrator;
use autoxide_kernel::storage::{ByteStream, Storage};
use autoxide_kernel::transfer::{CopiedTable, copy_modules};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings::{self, Settings};

const MANIFEST: &str = "backup.json";
const DATABASE_FILE: &str = "database.sqlite";
const FILES_DIR: &str = "files";
/// Phiên bản của định dạng thư mục backup.
const FORMAT: u64 = 1;

#[derive(Debug, Clone, clap::Args)]
pub struct BackupArguments {
    /// Directory to write the backup into; created when missing, must be empty.
    directory: PathBuf,
}

#[derive(Debug, Clone, clap::Args)]
pub struct RestoreArguments {
    /// Directory made by `autoxide backup`.
    directory: PathBuf,
}

pub fn backup(arguments: &BackupArguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    finish(run_with_observability(&config, run_backup(settings, arguments.directory.clone())))
}

pub fn restore(arguments: &RestoreArguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    finish(run_with_observability(&config, run_restore(settings, arguments.directory.clone())))
}

fn finish(outcome: Result<String, String>) -> ExitCode {
    match outcome {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => report_failure(&error),
    }
}

/// Tạo bản backup trong `directory`; trả nội dung in ra.
pub(crate) async fn run_backup(settings: Settings, directory: PathBuf) -> Result<String, String> {
    prepare_directory(&directory)?;
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let result = async {
        let migrator = runtime.modules.migrator().map_err(|error| error.to_string())?;
        let copy = connect_sqlite(&directory.join(DATABASE_FILE), &migrator).await?;
        let names = runtime.modules.names();
        let tables = copy_modules(&runtime.database, &copy, &names).await;
        copy.close().await.map_err(|error| format!("backup database: {error}"))?;
        let tables = tables.map_err(|error| error.to_string())?;
        let (files, bytes) = match runtime.modules.api::<dyn Storage>() {
            Some(storage) => save_files(storage, &directory.join(FILES_DIR)).await?,
            None => (0, 0),
        };
        let manifest = json!({
            "format": FORMAT,
            "autoxide": env!("CARGO_PKG_VERSION"),
            "created_at": chrono::Utc::now(),
            "source": format!("{:?}", runtime.database.backend()).to_lowercase(),
            "tables": tables.iter().map(|table| json!({"table": table.table, "rows": table.rows})).collect::<Vec<_>>(),
            "files": files,
            "file_bytes": bytes,
        });
        let text = serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
        tokio::fs::write(directory.join(MANIFEST), text)
            .await
            .map_err(|error| format!("cannot write {MANIFEST}: {error}"))?;
        Ok(summary("Backed up", &tables, files, &directory))
    }
    .await;
    let _ = runtime.database.close().await;
    result
}

/// Đưa bản backup trong `directory` vào database (mới, trống) và nơi lưu file đang cấu
/// hình; trả nội dung in ra.
pub(crate) async fn run_restore(settings: Settings, directory: PathBuf) -> Result<String, String> {
    let manifest = read_manifest(&directory).await?;
    let migrator = settings.modules.migrator().map_err(|error| error.to_string())?;
    let target = Database::connect(&settings.config.database).await.map_err(|e| e.to_string())?;
    let prepared = async {
        let states = migrator.status(&target).await.map_err(|error| error.to_string())?;
        if states.iter().any(|state| state.applied) {
            return Err("the database already has Autoxide tables; restore into a new, empty \
                        database"
                .to_owned());
        }
        migrator.up(&target).await.map_err(|error| error.to_string())
    }
    .await;
    let _ = target.close().await;
    prepared?;

    // Làm trên một bản sao của file backup: migration mới hơn (nếu có) được áp dụng lên
    // bản sao, bản backup giữ nguyên. Chỉ chép đúng `database.sqlite`, KHÔNG chép file
    // `-wal` / `-shm` nằm cạnh (backup đóng database trước khi xong nên không có): lỗ hổng
    // RUSTSEC-2026-0459 của SQLite đóng gói chỉ khai thác được qua một file `-wal` do kẻ
    // tấn công chuẩn bị (D-032, D-075).
    let scratch = std::env::temp_dir().join(format!("autoxide-restore-{}", uuid::Uuid::now_v7()));
    private_dir(&scratch).map_err(|error| error.to_string())?;
    let result = async {
        let copy = scratch.join(DATABASE_FILE);
        std::fs::copy(directory.join(DATABASE_FILE), &copy)
            .map_err(|error| format!("cannot read {DATABASE_FILE} of the backup: {error}"))?;
        let source = connect_sqlite(&copy, &migrator).await?;
        let runtime = match runtime::start(settings, Migrations::RequireApplied).await {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = source.close().await;
                return Err(error);
            }
        };
        let names = runtime.modules.names();
        let tables = copy_modules(&source, &runtime.database, &names).await;
        let _ = source.close().await;
        let restored = async {
            let tables = tables.map_err(|error| error.to_string())?;
            let files = match runtime.modules.api::<dyn Storage>() {
                Some(storage) => load_files(storage, &directory.join(FILES_DIR)).await?,
                None => 0,
            };
            Ok::<_, String>((tables, files))
        }
        .await;
        let _ = runtime.database.close().await;
        let (tables, files) = restored?;
        let expected = manifest["files"].as_u64().unwrap_or_default();
        if files != expected {
            return Err(format!(
                "the backup lists {expected} files but {files} were found in its `{FILES_DIR}` \
                 directory"
            ));
        }
        Ok(summary("Restored", &tables, files, &directory))
    }
    .await;
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn summary(verb: &str, tables: &[CopiedTable], files: u64, directory: &Path) -> String {
    let rows: u64 = tables.iter().map(|table| table.rows).sum();
    format!(
        "{verb} {rows} rows in {} tables and {files} files ({}).\n",
        tables.len(),
        directory.display()
    )
}

/// Thư mục đích phải chưa có hoặc trống.
fn prepare_directory(directory: &Path) -> Result<(), String> {
    if directory.exists() {
        let mut entries = std::fs::read_dir(directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        if entries.next().is_some() {
            return Err(format!(
                "{} is not empty; back up into a new or empty directory",
                directory.display()
            ));
        }
    }
    private_dir(directory).map_err(|error| format!("{}: {error}", directory.display()))
}

/// Tạo thư mục `path` (và các thư mục cha) rồi, trên Unix, chỉ để chủ của nó đọc / ghi / mở
/// (`0700`). Backup chứa hash mật khẩu, secret webhook, MFA, và khoá ký khi nó nằm trong
/// database: người dùng khác trên cùng máy không được đọc (D-158). Khoá thư mục là đủ — file bên
/// trong không mở được qua thư mục đã khoá.
fn private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

async fn read_manifest(directory: &Path) -> Result<Value, String> {
    let path = directory.join(MANIFEST);
    let text = tokio::fs::read_to_string(&path).await.map_err(|error| {
        format!("{} is not a backup made by `autoxide backup`: {error}", directory.display())
    })?;
    let manifest: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is damaged: {error}", path.display()))?;
    if manifest["format"].as_u64() != Some(FORMAT) {
        return Err(format!(
            "{} has backup format {}, this version reads format {FORMAT}",
            path.display(),
            manifest["format"]
        ));
    }
    Ok(manifest)
}

/// File SQLite ở `path`, đã áp dụng mọi migration.
async fn connect_sqlite(path: &Path, migrator: &Migrator) -> Result<Database, String> {
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let source = ConfigSource::from_pairs([("DATABASE_URL", url.as_str())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let (config, _) = reader.finish(config).map_err(|errors| errors.to_string())?;
    let database = Database::connect(&config).await.map_err(|error| error.to_string())?;
    if let Err(error) = migrator.up(&database).await {
        let _ = database.close().await;
        return Err(error.to_string());
    }
    Ok(database)
}

/// Chép mọi file từ nơi lưu vào `directory`; trả số file và số byte.
async fn save_files(storage: Arc<dyn Storage>, directory: &Path) -> Result<(u64, u64), String> {
    let mut files = 0;
    let mut bytes = 0;
    for key in storage.keys().await.map_err(describe)? {
        // File bị xoá trong lúc backup: bỏ qua.
        let Some(stored) = storage.get(&key).await.map_err(describe)? else { continue };
        let path = directory.join(&key);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| error.to_string())?;
        }
        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let mut content = stored.content;
        while let Some(chunk) = content.next().await {
            let chunk = chunk.map_err(describe)?;
            bytes += chunk.len() as u64;
            file.write_all(&chunk).await.map_err(|error| error.to_string())?;
        }
        file.sync_all().await.map_err(|error| error.to_string())?;
        files += 1;
    }
    Ok((files, bytes))
}

/// Đưa mọi file trong `directory` (khoá = đường dẫn tương đối) vào nơi lưu.
async fn load_files(storage: Arc<dyn Storage>, directory: &Path) -> Result<u64, String> {
    let mut files = 0;
    let mut pending = vec![directory.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let mut entries = match tokio::fs::read_dir(&folder).await {
            Ok(entries) => entries,
            // Bản backup không có file nào.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", folder.display())),
        };
        while let Some(entry) = entries.next_entry().await.map_err(|error| error.to_string())? {
            let path = entry.path();
            let kind = entry.file_type().await.map_err(|error| error.to_string())?;
            // Bản backup chỉ chứa file thường và thư mục. Symlink có thể trỏ ra ngoài (vd
            // một file hệ thống) và đưa nó vào kho lưu, nên từ chối (D-079).
            if kind.is_symlink() {
                return Err(format!(
                    "{} is a symbolic link; a backup made by `autoxide backup` has none",
                    path.display()
                ));
            }
            if kind.is_dir() {
                pending.push(path);
                continue;
            }
            let relative = path.strip_prefix(directory).map_err(|error| error.to_string())?;
            let key: Vec<String> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            storage.put(&key.join("/"), read_file(path).await?).await.map_err(describe)?;
            files += 1;
        }
    }
    Ok(files)
}

/// Nội dung file `path` dưới dạng luồng, đọc từng đoạn 64 KiB.
async fn read_file(path: PathBuf) -> Result<ByteStream, String> {
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|error| format!("{}: {error}", path.display()))?;
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

/// Lỗi kèm nguyên nhân gốc (lỗi nội bộ chỉ có câu chung chung).
fn describe(error: Error) -> String {
    match std::error::Error::source(&error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => error.message(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_auth::api::{Accounts, NewAccount};
    use autoxide_kernel::tenancy::TenantContext;
    use autoxide_testkit::TestDatabase;
    use futures_util::TryStreamExt as _;

    /// Thư mục tạm của một bài test, xoá khi xong.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("autoxide-backup-{}", uuid::Uuid::now_v7()));
            std::fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn settings(database_url: &str, files: &Path) -> Settings {
        let files = files.display().to_string();
        let source = ConfigSource::from_pairs([
            ("DATABASE_URL", database_url),
            ("AUTOXIDE_FILES_DIR", files.as_str()),
        ]);
        crate::settings::read(&source).unwrap()
    }

    fn admin(email: &str) -> NewAccount {
        NewAccount {
            email: email.to_owned(),
            name: "Admin".to_owned(),
            password: Some("correct horse battery staple".to_owned()),
            admin: true,
        }
    }

    /// Database có một quản trị viên và hai file.
    async fn seed(database_url: &str, files: &Path) {
        let settings = settings(database_url, files);
        let database = Database::connect(&settings.config.database).await.unwrap();
        settings.modules.migrator().unwrap().up(&database).await.unwrap();
        database.close().await.unwrap();
        let runtime = runtime::start(settings, Migrations::RequireApplied).await.unwrap();
        let accounts = runtime.modules.api::<dyn Accounts>().unwrap();
        accounts
            .create(&TenantContext::default_on(runtime.database.clone()), admin("root@example.com"))
            .await
            .unwrap();
        let storage = runtime.modules.api::<dyn Storage>().unwrap();
        for (key, text) in [("files/a", "first file"), ("files/b/c", "second file")] {
            let chunks = vec![Ok(bytes::Bytes::from(text))];
            storage.put(key, futures_util::stream::iter(chunks).boxed()).await.unwrap();
        }
        runtime.database.close().await.unwrap();
    }

    /// Dữ liệu đã về đủ: tài khoản có mặt (tạo lại thì trùng), file đúng nội dung.
    async fn check(database_url: &str, files: &Path) {
        let runtime = runtime::start(settings(database_url, files), Migrations::RequireApplied)
            .await
            .unwrap();
        let accounts = runtime.modules.api::<dyn Accounts>().unwrap();
        let duplicate = accounts
            .create(&TenantContext::default_on(runtime.database.clone()), admin("root@example.com"))
            .await
            .unwrap_err();
        assert_eq!(duplicate.code(), "auth.email_taken");
        let storage = runtime.modules.api::<dyn Storage>().unwrap();
        assert_eq!(storage.keys().await.unwrap(), ["files/a", "files/b/c"]);
        let file = storage.get("files/b/c").await.unwrap().unwrap();
        let chunks: Vec<bytes::Bytes> = file.content.try_collect().await.unwrap();
        assert_eq!(chunks.concat(), b"second file");
        runtime.database.close().await.unwrap();
    }

    /// Nguồn là database gate đang chọn; khôi phục vào một file SQLite mới.
    /// Thư mục backup chỉ chủ của nó mở được (D-158).
    #[cfg(unix)]
    #[test]
    fn thu_muc_backup_chi_chu_mo_duoc() {
        use std::os::unix::fs::PermissionsExt as _;
        let path = std::env::temp_dir().join(format!("autoxide-private-{}", uuid::Uuid::now_v7()));
        private_dir(&path.join("inner")).unwrap();
        let mode = std::fs::metadata(path.join("inner")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[tokio::test]
    async fn backup_roi_restore_vao_sqlite() {
        TestDatabase::run(async |test| {
            let scratch = Scratch::new();
            seed(test.url(), &scratch.0.join("files-a")).await;
            let backup = scratch.0.join("backup");

            let report =
                run_backup(settings(test.url(), &scratch.0.join("files-a")), backup.clone())
                    .await
                    .unwrap();
            assert!(report.starts_with("Backed up "), "{report}");
            assert!(report.contains(" and 2 files"), "{report}");

            let restored =
                format!("sqlite://{}?mode=rwc", scratch.0.join("restored.sqlite").display());
            let files = scratch.0.join("files-b");
            let report = run_restore(settings(&restored, &files), backup.clone()).await.unwrap();
            assert!(report.starts_with("Restored "), "{report}");
            check(&restored, &files).await;

            // Database đích đã có dữ liệu, hay thư mục backup đã có gì: từ chối.
            let again = run_restore(settings(&restored, &files), backup.clone()).await.unwrap_err();
            assert!(again.contains("restore into a new, empty database"), "{again}");
            let full = run_backup(settings(test.url(), &scratch.0.join("files-a")), backup)
                .await
                .unwrap_err();
            assert!(full.contains("is not empty"), "{full}");
        })
        .await
        .unwrap();
    }

    /// Nguồn là một file SQLite; khôi phục vào database gate đang chọn (lượt Postgres
    /// của gate: SQLite → Postgres).
    #[tokio::test]
    async fn backup_tu_sqlite_restore_vao_database_cua_gate() {
        TestDatabase::run(async |test| {
            let scratch = Scratch::new();
            let source = format!("sqlite://{}?mode=rwc", scratch.0.join("source.sqlite").display());
            seed(&source, &scratch.0.join("files-a")).await;
            let backup = scratch.0.join("backup");
            run_backup(settings(&source, &scratch.0.join("files-a")), backup.clone())
                .await
                .unwrap();

            let files = scratch.0.join("files-b");
            run_restore(settings(test.url(), &files), backup.clone()).await.unwrap();

            check(test.url(), &files).await;
            // Bản backup bị cài symlink trỏ ra ngoài: từ chối, không đọc file đó.
            let tampered = scratch.0.join("tampered");
            std::fs::create_dir_all(&tampered).unwrap();
            for entry in ["backup.json", "database.sqlite"] {
                std::fs::copy(backup.join(entry), tampered.join(entry)).unwrap();
            }
            std::fs::create_dir_all(tampered.join("files/files")).unwrap();
            std::os::unix::fs::symlink("/etc/hostname", tampered.join("files/files/x")).unwrap();
            let target = format!("sqlite://{}?mode=rwc", scratch.0.join("t.sqlite").display());
            let refused =
                run_restore(settings(&target, &scratch.0.join("files-c")), tampered).await;
            assert!(refused.unwrap_err().contains("is a symbolic link"));
            // Thư mục không phải bản backup: báo rõ.
            let wrong = run_restore(settings(test.url(), &files), scratch.0.clone()).await;
            assert!(wrong.unwrap_err().contains("is not a backup made by `autoxide backup`"));
        })
        .await
        .unwrap();
    }

    /// Backup rồi restore mang theo cả thông tin (bảng `media_…`) lẫn nội dung (khoá
    /// `media/…`) của thư viện media.
    #[tokio::test]
    async fn backup_mang_theo_media() {
        use autoxide_media::api::{Media, MediaVisibility};

        TestDatabase::run(async |test| {
            let scratch = Scratch::new();
            let files_a = scratch.0.join("files-a");
            let storage = [("AUTOXIDE_FILES_DIR", files_a.display().to_string())];
            let runtime = crate::e2e::media::start(test.url(), &storage).await;
            let token = crate::e2e::media::admin_token(&runtime, "root@example.com").await;
            let auth = format!("Bearer {token}");
            let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR\x00\x00\x00\x07\x00\x00\x00\x05\x08\x06\x00\x00\x00\x00\x00\x00\x00";
            let headers = [("authorization", auth.as_str()), ("content-type", "image/png")];
            let (_, _, uploaded) =
                crate::e2e::media::call(&runtime, "POST", "/api/v1/media?name=a.png", &headers, png.to_vec())
                    .await;
            let id: uuid::Uuid = uploaded["data"]["id"].as_str().unwrap().parse().unwrap();
            let headers = [
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("if-match", "*"),
            ];
            let change = br#"{"alt": "Seven by five", "visibility": "public"}"#.to_vec();
            let (status, _, _) =
                crate::e2e::media::call(&runtime, "PATCH", &format!("/api/v1/media/{id}"), &headers, change)
                    .await;
            assert_eq!(status, axum::http::StatusCode::OK);
            runtime.database.close().await.unwrap();

            let backup = scratch.0.join("backup");
            run_backup(settings(test.url(), &files_a), backup.clone()).await.unwrap();
            let restored =
                format!("sqlite://{}?mode=rwc", scratch.0.join("restored.sqlite").display());
            let files_b = scratch.0.join("files-b");
            run_restore(settings(&restored, &files_b), backup).await.unwrap();

            let runtime = runtime::start(settings(&restored, &files_b), Migrations::RequireApplied)
                .await
                .unwrap();
            let media = runtime.modules.api::<dyn Media>().unwrap();
            let item = media.find(&TenantContext::default_on(runtime.database.clone()), id).await.unwrap().unwrap();
            assert_eq!((item.width, item.height, item.alt.as_str()), (Some(7), Some(5), "Seven by five"));
            assert_eq!(item.visibility, MediaVisibility::Public);
            let storage = runtime.modules.api::<dyn Storage>().unwrap();
            let keys = storage.keys_with_prefix("media/").await.unwrap();
            assert_eq!(keys.len(), 1);
            assert!(item.url.ends_with(keys[0].rsplit('/').next().unwrap()));
            let file = storage.get(&keys[0]).await.unwrap().unwrap();
            let chunks: Vec<bytes::Bytes> = file.content.try_collect().await.unwrap();
            assert_eq!(chunks.concat(), png);
            runtime.database.close().await.unwrap();
        })
        .await
        .unwrap();
    }
}
