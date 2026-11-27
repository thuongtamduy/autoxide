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

use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::storage::Storage;
use autoxide_kernel::transfer::{CopiedTable, copy_modules};
use serde_json::Value;

use super::{report_failure, report_invalid_configuration, run_with_observability};
use crate::runtime::{self, Migrations};
use crate::settings::{self, Settings};

use autoxide_kernel::backup::{
    self, DATABASE_FILE, FILES_DIR, FORMAT, MANIFEST, open_sqlite, private_dir, read_file,
};

#[derive(Debug, Clone, clap::Args)]
pub struct BackupArguments {
    /// Directory to write the backup into; created when missing, must be empty.
    directory: PathBuf,
    /// Also back up every tenant database (placement), and its own file storage, into
    /// `placements/<name>/` of the directory (multi-tenant mode).
    #[arg(long)]
    all: bool,
}

#[derive(Debug, Clone, clap::Args)]
pub struct RestoreArguments {
    /// Directory made by `autoxide backup` (for `--placement`: one made with `--all`, or its
    /// `placements/<name>/`).
    directory: PathBuf,
    /// Restore the backup of this tenant database (placement) into that same placement, whose database
    /// must be empty again, instead of the main database; the files go to its own file storage, or the
    /// shared one.
    #[arg(long)]
    placement: Option<String>,
}

pub fn backup(arguments: &BackupArguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    finish(run_with_observability(
        &config,
        run_backup(settings, arguments.directory.clone(), arguments.all),
    ))
}

pub fn restore(arguments: &RestoreArguments) -> ExitCode {
    let settings = match settings::load() {
        Ok(settings) => settings,
        Err(errors) => return report_invalid_configuration(&errors),
    };
    let config = settings.config.clone();
    let directory = arguments.directory.clone();
    match arguments.placement.clone() {
        Some(name) => finish(run_with_observability(
            &config,
            super::backup_placements::restore_placement(settings, directory, name),
        )),
        None => finish(run_with_observability(&config, run_restore(settings, directory))),
    }
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

/// Tạo bản backup trong `directory`; trả nội dung in ra. Lõi nằm ở `autoxide_kernel::backup`, dùng
/// chung với job backup theo lịch.
pub(crate) async fn run_backup(
    settings: Settings,
    directory: PathBuf,
    all: bool,
) -> Result<String, String> {
    // Đường dẫn tuyệt đối: người chạy trong container phải thấy bản backup nằm ở ĐÂU.
    let directory = absolute(&directory);
    prepare_directory(&directory)?;
    if let Some(warning) = container_warning(&directory) {
        eprintln!("autoxide: warning: {warning}");
    }
    // Không đòi database đủ migration: backup trước khi nâng cấp là lúc database còn ở bản cũ. Bản backup giữ đúng
    // schema của nguồn (D-20261126-upgrade-rollback), nên chương trình của bản đó restore được.
    let runtime = runtime::start(settings, Migrations::Ignore).await?;
    let result = async {
        let migrator = runtime.modules.migrator().map_err(|error| error.to_string())?;
        let behind = migrations_behind(&migrator, &runtime.database).await?;
        // Module đang tắt còn dữ liệu: vẫn từ chối (bản backup chỉ chứa module đang bật) cho tới khi backup mang theo
        // được bảng của module tắt (bước 70 P9).
        crate::modules::refuse_dormant_data(&runtime.modules, &runtime.database, "backup").await?;
        let names = runtime.modules.names();
        // `--all`: các placement trước, manifest của database chính SAU CÙNG kèm danh sách placement — manifest chính
        // có mặt nghĩa là mọi placement nó nêu đã xong.
        let (mut placements_text, placements) = if all {
            super::backup_placements::write_all(&runtime, &names, &directory).await.map_err(|error| {
                // Manifest chính chưa ghi: thư mục không phải bản backup trọn, restore sẽ từ chối nó.
                format!(
                    "{error}. The backup is incomplete ({} has no {MANIFEST}): remove the directory and run \
                     the command again",
                    directory.display()
                )
            })?
        } else {
            (skipped_placements(runtime.modules.tenancy().as_ref()).await?, Vec::new())
        };
        let scratch =
            std::env::temp_dir().join(format!("autoxide-backup-{}", uuid::Uuid::now_v7()));
        let manifest = backup::write_backup_of(
            &runtime.database,
            &migrator,
            &names,
            runtime.modules.api::<dyn Storage>(),
            &backup::DirectorySink::new(&directory),
            &scratch,
            backup::BackupOf::main(placements),
        )
        .await
        .map_err(describe)?;
        let mut text = format!(
            "Backed up {} rows in {} tables and {} files ({}).\n",
            manifest.rows(),
            manifest.tables.len(),
            manifest.files,
            directory.display()
        );
        if behind > 0 {
            text.push_str(&format!(
                "Note: the database is {behind} migration(s) behind this program, and the backup keeps it that \
                 way: restore it with the release the database runs, or this one.\n"
            ));
        }
        text.push_str(&std::mem::take(&mut placements_text));
        Ok(text)
    }
    .await;
    let _ = runtime.database.close().await;
    result
}

/// Số migration của chương trình này chưa áp dụng vào `database`. Database đã được một bản Autoxide MỚI hơn
/// migrate (có migration chương trình này không biết): báo rõ thay vì lỗi nội bộ.
pub(crate) async fn migrations_behind(
    migrator: &autoxide_kernel::migration::Migrator,
    database: &Database,
) -> Result<usize, String> {
    match migrator.status(database).await {
        Ok(states) => Ok(states.iter().filter(|state| !state.applied).count()),
        Err(error) if error.to_string().contains("its file is missing") => Err(format!(
            "the database was migrated by a newer Autoxide than this one ({}); back it up with that version",
            env!("CARGO_PKG_VERSION")
        )),
        Err(error) => Err(error.to_string()),
    }
}

/// Chế độ `multi`: backup chỉ chép database chính (sổ khách và khách trên placement chung) và mọi
/// file ở nơi lưu chung; database riêng của placement khác và nơi lưu file riêng của placement KHÔNG
/// được chép (bước 39 làm đủ). Trả lời cảnh báo để in
/// (rỗng khi không có placement nào khác), và ghi log.
async fn skipped_placements(
    registry: Option<&autoxide_kernel::tenancy::DatabaseRegistry>,
) -> Result<String, String> {
    let Some(registry) = registry else { return Ok(String::new()) };
    if registry.mode() != autoxide_kernel::tenancy::TenancyMode::Multi {
        return Ok(String::new());
    }
    let others: Vec<String> = registry
        .databases()
        .await
        .map_err(describe)?
        .into_iter()
        .filter(|(id, _, _)| *id != autoxide_kernel::tenancy::MAIN_PLACEMENT)
        .map(|(_, name, _)| name)
        .collect();
    if others.is_empty() {
        return Ok(String::new());
    }
    tracing::warn!(placements = others.join(","), "backup does not copy dedicated placements");
    Ok(format!(
        "WARNING: only the main database was copied. The databases of these placements were NOT: \
         {}. Run `autoxide backup --all` to back them up too, with their own file storage.\n",
        others.join(", ")
    ))
}

/// Đưa bản backup trong `directory` vào database (mới, trống) và nơi lưu file đang cấu
/// hình; trả nội dung in ra.
pub(crate) async fn run_restore(settings: Settings, directory: PathBuf) -> Result<String, String> {
    let manifest = read_manifest(&directory).await?;
    if let Some(name) = manifest["placement"]["name"].as_str() {
        return Err(format!(
            "this is the backup of placement `{name}`, not of the main database; restore it with \
             `autoxide restore <directory> --placement {name}`"
        ));
    }
    let migrator = settings.modules.migrator().map_err(|error| error.to_string())?;
    refuse_newer_backup(&manifest, &migrator)?;
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
        let source = open_sqlite(&copy, &migrator).await.map_err(describe)?;
        let runtime = match runtime::start(settings, Migrations::RequireApplied).await {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = source.close().await;
                return Err(error);
            }
        };
        let names = runtime.modules.names();
        // Bản backup có dữ liệu của module mà bản cài này đang tắt: dừng, không bỏ sót nó.
        let tables =
            match crate::modules::refuse_dormant_data(&runtime.modules, &source, "restore").await {
                Ok(()) => copy_modules(&source, &runtime.database, &names)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            };
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
        let mut text = summary("Restored", &tables, files, &directory);
        text.push_str(&placements_to_restore(&manifest));
        Ok(text)
    }
    .await;
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// Bản backup do một bản Autoxide MỚI hơn làm (module đang chạy ở đây mà có migration chương trình này không biết):
/// từ chối rõ, TRƯỚC khi chạm database đích. Migration của module đang tắt ở đây thì bỏ qua: `refuse_dormant_data`
/// trên bản sao quyết (chỉ dừng khi module đó có dữ liệu). Bản backup cũ (định dạng 1 trước khi có danh sách
/// migration) không có gì để kiểm.
pub(crate) fn refuse_newer_backup(
    manifest: &Value,
    migrator: &autoxide_kernel::migration::Migrator,
) -> Result<(), String> {
    let known: std::collections::BTreeSet<String> = migrator.names().into_iter().collect();
    let running: std::collections::BTreeSet<&str> =
        known.iter().filter_map(|name| name.split('/').next()).collect();
    let listed = manifest["migrations"].as_array().map(Vec::as_slice).unwrap_or_default();
    let newer: Vec<&str> = listed
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| !known.contains(*name))
        .filter(|name| running.contains(name.split('/').next().unwrap_or(name)))
        .collect();
    if !newer.is_empty() {
        let shown = newer.iter().take(3).copied().collect::<Vec<_>>().join(", ");
        return Err(format!(
            "this backup holds {} migration(s) this Autoxide ({}) does not know ({shown}): it was made from a \
             database migrated by a newer release (`backup.json` names the version that wrote it); restore it \
             with that release",
            newer.len(),
            env!("CARGO_PKG_VERSION"),
        ));
    }
    Ok(())
}

/// Bản backup làm bằng `--all`: nhắc restore từng placement nó chứa.
fn placements_to_restore(manifest: &Value) -> String {
    let names: Vec<&str> = manifest["placements"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    if names.is_empty() {
        return String::new();
    }
    format!(
        "The backup also holds these placements: {}. Restore each one into its own empty database with \
         `autoxide restore <directory> --placement <name>`.\n",
        names.join(", ")
    )
}

pub(crate) fn summary(verb: &str, tables: &[CopiedTable], files: u64, directory: &Path) -> String {
    let directory = absolute(directory);
    let directory = directory.as_path();
    let rows: u64 = tables.iter().map(|table| table.rows).sum();
    format!(
        "{verb} {rows} rows in {} tables and {files} files ({}).\n",
        tables.len(),
        directory.display()
    )
}

/// Đường dẫn tuyệt đối (không cần thư mục tồn tại); không đổi được thì giữ nguyên.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Thư mục đích phải chưa có hoặc trống.
pub(crate) fn prepare_directory(directory: &Path) -> Result<(), String> {
    if directory.exists() {
        let mut entries =
            std::fs::read_dir(directory).map_err(|error| describe_io(directory, &error))?;
        if entries.next().is_some() {
            return Err(format!(
                "{} is not empty; back up into a new or empty directory",
                directory.display()
            ));
        }
    }
    private_dir(directory).map_err(|error| describe_io(directory, &error))
}

/// Lỗi ghi/đọc thư mục; lỗi quyền (`os error 13`) kèm cách sửa, vì người dùng container hay
/// gặp: image chạy bằng user không phải root (10001) nên không ghi được vào thư mục của root.
fn describe_io(directory: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        return format!(
            "{}: permission denied. The user running autoxide (in the container image: uid \
             10001) must be able to write there: create the directory first and give it to that \
             user (`mkdir -p <dir> && sudo chown 10001 <dir>`), or choose another directory",
            directory.display()
        );
    }
    format!("{}: {error}", directory.display())
}

/// Cảnh báo (không chặn) khi chạy trong container mà `directory` nằm cùng thiết bị với `/`:
/// bản backup sẽ nằm trong lớp ghi của container và mất khi container bị xoá.
fn container_warning(directory: &Path) -> Option<String> {
    let in_container =
        Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists();
    let on_root = same_device(directory, Path::new("/"))?;
    warn_unmounted(in_container, on_root, directory)
}

/// Phần quyết định của [`container_warning`], tách ra để test không cần container thật.
fn warn_unmounted(in_container: bool, on_root_device: bool, directory: &Path) -> Option<String> {
    (in_container && on_root_device).then(|| {
        format!(
            "{} is inside the container's own filesystem, not on a volume or bind mount; the \
             backup is lost when the container is removed. Mount a volume (for example \
             `-v \"$PWD/backups:/backups\"`) and back up into it",
            directory.display()
        )
    })
}

/// `a` và `b` cùng thiết bị? `None` khi không xác định được (hệ không phải Unix, hay lỗi đọc).
#[cfg(unix)]
fn same_device(a: &Path, b: &Path) -> Option<bool> {
    use std::os::unix::fs::MetadataExt as _;
    Some(std::fs::metadata(a).ok()?.dev() == std::fs::metadata(b).ok()?.dev())
}

#[cfg(not(unix))]
fn same_device(_: &Path, _: &Path) -> Option<bool> {
    None
}

pub(crate) async fn read_manifest(directory: &Path) -> Result<Value, String> {
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

/// Đưa mọi file trong `directory` (khoá = đường dẫn tương đối) vào nơi lưu.
pub(crate) async fn load_files(storage: Arc<dyn Storage>, directory: &Path) -> Result<u64, String> {
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
            let content = read_file(path).await.map_err(describe)?;
            storage.put(&key.join("/"), content).await.map_err(describe)?;
            files += 1;
        }
    }
    Ok(files)
}

/// Lỗi kèm nguyên nhân gốc (lỗi nội bộ chỉ có câu chung chung).
pub(crate) fn describe(error: Error) -> String {
    match std::error::Error::source(&error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => error.message(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_auth::api::{Accounts, NewAccount};
    use autoxide_kernel::config::ConfigSource;
    use autoxide_kernel::tenancy::TenantContext;
    use autoxide_testkit::TestDatabase;
    use futures_util::{StreamExt as _, TryStreamExt as _};

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
        autoxide_testkit::prepare_from_template(
            &settings.config.database,
            &settings.modules.migrator().unwrap(),
        )
        .await;
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

    /// Chế độ `multi` có placement riêng: không `--all` thì backup nói rõ database của placement không được chép;
    /// `--all` chép nó vào `placements/<tên>/` (manifest chính ghi sau cùng, nêu tên nó). Phục hồi sau sự cố: database của
    /// placement trống lại ở đúng URL, restore vào đúng tên. Bản backup không đúng database thì bị từ chối.
    #[tokio::test]
    async fn multi_backup_all_roi_restore_placement() {
        use autoxide_kernel::tenancy::{
            DatabaseRegistry, NewPlacement, PlacementId, PlacementKind, TenancyConfig, TenancyMode,
        };
        TestDatabase::run(async |test| {
            let scratch = Scratch::new();
            let files = scratch.0.join("files");
            let multi = |url: &str| {
                let files = files.display().to_string();
                let source = ConfigSource::from_pairs([
                    ("DATABASE_URL", url),
                    ("AUTOXIDE_FILES_DIR", files.as_str()),
                    ("AUTOXIDE_TENANCY_MODE", "multi"),
                    ("AUTOXIDE_SECRET_KEY", "a-configured-secret-that-is-long-enough-0123"),
                ]);
                crate::settings::read(&source).unwrap()
            };
            let settings = multi(test.url());
            let database = Database::connect(&settings.config.database).await.unwrap();
            let single = DatabaseRegistry::new(
                database.clone(),
                settings.config.database.clone(),
                autoxide_kernel::tenancy::TenancyConfig::default(),
            );
            assert_eq!(skipped_placements(Some(&single)).await.unwrap(), "");
            let registry = DatabaseRegistry::new(
                database.clone(),
                settings.config.database.clone(),
                TenancyConfig::with_mode(TenancyMode::Multi),
            );
            let placements = settings.modules.placements(&registry).unwrap();
            placements.up_all().await.unwrap();
            for name in ["eu-1", "eu-2"] {
                let url = test.extra_database().await.unwrap();
                let new = NewPlacement::new(name, PlacementKind::Dedicated, url.parse().unwrap());
                placements.add(new).await.unwrap();
            }
            placements.up_all().await.unwrap();
            let warning = skipped_placements(Some(&registry)).await.unwrap();
            assert!(warning.contains("`autoxide backup --all`"), "{warning}");

            let backup = scratch.0.join("backup");
            let report = run_backup(multi(test.url()), backup.clone(), true).await.unwrap();
            assert!(report.contains("Backed up placement eu-1: "), "{report}");
            let read = |path: &std::path::Path| -> Value {
                serde_json::from_str(&std::fs::read_to_string(path.join(MANIFEST)).unwrap())
                    .unwrap()
            };
            let main = read(&backup);
            assert_eq!(main["placements"], serde_json::json!(["eu-1", "eu-2"]), "{main}");
            assert!(main["placement"].is_null(), "{main}");
            let eu1 = backup.join("placements/eu-1");
            let manifest = read(&eu1);
            assert_eq!(manifest["placement"]["name"], "eu-1", "{manifest}");
            assert!(!manifest["migrations"].as_array().unwrap().is_empty(), "{manifest}");

            // Database của eu-1 "mất": trống lại (mọi migration được gỡ), cùng URL.
            let id =
                placements.list().await.unwrap().into_iter().find(|p| p.name == "eu-1").unwrap().id;
            let eu1_database =
                registry.placement_database(PlacementId::from_uuid(id)).await.unwrap();
            let migrator = settings.modules.migrator().unwrap().keyed_on(&database);
            migrator.rollback(&eu1_database, &|_, _| false).await.unwrap();

            let restore = |directory: PathBuf, name: &str| {
                crate::commands::backup_placements::restore_placement(
                    multi(test.url()),
                    directory,
                    name.to_owned(),
                )
            };
            // Bản của eu-1 vào eu-2, bản chính vào một placement, bản của placement vào database chính: từ chối.
            let wrong = restore(eu1.clone(), "eu-2").await.unwrap_err();
            assert!(
                wrong.contains("the backup of placement `eu-1`, not of placement `eu-2`"),
                "{wrong}"
            );
            let wrong = run_restore(multi(test.url()), eu1.clone()).await.unwrap_err();
            assert!(wrong.contains("--placement eu-1"), "{wrong}");
            // Bản backup không chứa placement được hỏi: từ chối, nêu những placement nó có.
            let wrong = restore(backup.clone(), "eu-9").await.unwrap_err();
            assert!(
                wrong.contains("only these placements: eu-1, eu-2, not placement `eu-9`"),
                "{wrong}"
            );

            // Thư mục của bản backup chính (`--all`) cũng được: lấy `placements/eu-1/` của nó.
            let restored = restore(backup.clone(), "eu-1").await.unwrap();
            assert!(restored.starts_with("Restored placement eu-1:"), "{restored}");
            let state =
                placements.list().await.unwrap().into_iter().find(|p| p.name == "eu-1").unwrap();
            assert_eq!(state.schema_state, "ready");
            // Placement đã có bảng: từ chối.
            let again = restore(eu1, "eu-1").await.unwrap_err();
            assert!(again.contains("already has Autoxide tables"), "{again}");
            database.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    /// Database đang ở bản cũ (thiếu migration mới nhất của `auth`): backup không đòi migrate trước, giữ đúng
    /// schema nguồn (`backup.json` ghi đúng danh sách đã áp), và restore bằng chương trình này vẫn được.
    #[tokio::test]
    async fn backup_database_chua_migrate_giu_schema_nguon() {
        TestDatabase::run(async |test| {
            let scratch = Scratch::new();
            let files_a = scratch.0.join("files-a");
            seed(test.url(), &files_a).await;
            let migrator = settings(test.url(), &files_a).modules.migrator().unwrap();
            let database =
                Database::connect(&settings(test.url(), &files_a).config.database).await.unwrap();
            let reverted = migrator.down(&database, "auth", 1).await.unwrap();
            let missing = format!("auth/{}", reverted[0].name);
            database.close().await.unwrap();

            let backup = scratch.0.join("backup");
            let report =
                run_backup(settings(test.url(), &files_a), backup.clone(), false).await.unwrap();
            assert!(report.contains("1 migration(s) behind this program"), "{report}");
            let manifest: Value =
                serde_json::from_str(&std::fs::read_to_string(backup.join(MANIFEST)).unwrap())
                    .unwrap();
            let listed: Vec<&str> = manifest["migrations"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            assert!(!listed.contains(&missing.as_str()), "{missing} must not be in the backup");
            assert_eq!(listed.len(), migrator.names().len() - 1);

            let restored =
                format!("sqlite://{}?mode=rwc", scratch.0.join("restored.sqlite").display());
            let files_b = scratch.0.join("files-b");
            run_restore(settings(&restored, &files_b), backup.clone()).await.unwrap();
            check(&restored, &files_b).await;

            // Database do một bản Autoxide MỚI hơn migrate (có migration chương trình này không biết): báo rõ.
            let database =
                Database::connect(&settings(test.url(), &files_a).config.database).await.unwrap();
            let backend = sea_orm::ConnectionTrait::get_database_backend(database.write());
            let future = autoxide_kernel::database::statement(
                backend,
                "INSERT INTO auth_migrations (version, applied_at) VALUES ($1, $2)",
                ["m9999_from_the_future".into(), 0i64.into()],
            );
            sea_orm::ConnectionTrait::execute_raw(database.write(), future).await.unwrap();
            database.close().await.unwrap();
            let newer = run_backup(settings(test.url(), &files_a), scratch.0.join("newer"), false)
                .await
                .unwrap_err();
            assert!(newer.contains("migrated by a newer Autoxide"), "{newer}");

            // Restore một bản backup do bản mới hơn làm: từ chối trước khi chạm database đích.
            let mut manifest: Value =
                serde_json::from_str(&std::fs::read_to_string(backup.join(MANIFEST)).unwrap())
                    .unwrap();
            manifest["migrations"]
                .as_array_mut()
                .unwrap()
                .push(Value::from("auth/m9999_from_the_future"));
            // Migration của module không chạy ở đây không làm bản backup thành "mới hơn".
            manifest["migrations"].as_array_mut().unwrap().push(Value::from("unknown/m0001_x"));
            std::fs::write(backup.join(MANIFEST), manifest.to_string()).unwrap();
            let empty = format!("sqlite://{}?mode=rwc", scratch.0.join("empty.sqlite").display());
            let refused =
                run_restore(settings(&empty, &files_b), backup.clone()).await.unwrap_err();
            assert!(refused.contains("1 migration(s) this Autoxide"), "{refused}");
            assert!(refused.contains("auth/m9999_from_the_future"), "{refused}");
            let target =
                Database::connect(&settings(&empty, &files_b).config.database).await.unwrap();
            let states = migrator.status(&target).await.unwrap();
            assert!(states.iter().all(|state| !state.applied), "the target must stay untouched");
            target.close().await.unwrap();
        })
        .await
        .unwrap();
    }

    #[test]
    fn canh_bao_chi_khi_trong_container_va_cung_thiet_bi_voi_goc() {
        let dir = Path::new("/data/b");
        assert!(warn_unmounted(true, true, dir).is_some_and(|text| text.contains("/data/b")));
        assert!(warn_unmounted(true, false, dir).is_none());
        assert!(warn_unmounted(false, true, dir).is_none());
    }

    #[test]
    fn loi_quyen_ghi_noi_cach_sua() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let text = describe_io(Path::new("/backups"), &error);
        assert!(text.contains("permission denied") && text.contains("chown 10001"), "{text}");
        assert!(!text.contains("os error"));
    }

    #[test]
    fn duong_dan_bao_cao_la_tuyet_doi() {
        assert!(absolute(Path::new("backups/x")).is_absolute());
    }

    #[tokio::test]
    async fn backup_roi_restore_vao_sqlite() {
        TestDatabase::run(async |test| {
            let scratch = Scratch::new();
            seed(test.url(), &scratch.0.join("files-a")).await;
            let backup = scratch.0.join("backup");

            let report =
                run_backup(settings(test.url(), &scratch.0.join("files-a")), backup.clone(), false)
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
            let full = run_backup(settings(test.url(), &scratch.0.join("files-a")), backup, false)
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
            run_backup(settings(&source, &scratch.0.join("files-a")), backup.clone(), false)
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
            run_backup(settings(test.url(), &files_a), backup.clone(), false).await.unwrap();
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
