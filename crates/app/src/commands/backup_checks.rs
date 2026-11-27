//! Kiểm một bản backup TRƯỚC khi restore chạm database đích (dùng chung cho database chính và placement): mọi lỗi có
//! thể biết trước (bản backup của bản Autoxide mới hơn, dữ liệu của module đang tắt hay không có trong bản này, thiếu
//! file) báo khi đích còn nguyên — chạy lại sau khi sửa cấu hình không vướng "đích đã có bảng".

use std::collections::BTreeSet;
use std::path::Path;

use autoxide_kernel::backup::{DATABASE_FILE, FILES_DIR, open_sqlite};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::Migrator;
use autoxide_kernel::module::Modules;
use serde_json::Value;

use super::backup::describe;

/// Bản sao của `database.sqlite` (trong `scratch`), đã nâng lên schema của chương trình này và đã qua mọi kiểm. Người
/// gọi đóng `Database` khi xong.
pub(crate) async fn open_checked(
    modules: &Modules,
    migrator: &Migrator,
    manifest: &Value,
    directory: &Path,
    scratch: &Path,
) -> Result<Database, String> {
    refuse_newer_backup(manifest, migrator)?;
    refuse_unknown_modules(manifest, migrator)?;
    let files = count_files(&directory.join(FILES_DIR))?;
    let expected = manifest["files"].as_u64().unwrap_or_default();
    if files != expected {
        return Err(format!(
            "the backup lists {expected} files but {files} were found in its `{FILES_DIR}` directory"
        ));
    }
    // Làm trên một bản sao của file backup: migration mới hơn (nếu có) được áp dụng lên bản sao, bản backup giữ
    // nguyên. Chỉ chép đúng `database.sqlite`, KHÔNG chép file `-wal` / `-shm` nằm cạnh (backup đóng database trước
    // khi xong nên không có): lỗ hổng RUSTSEC-2026-0459 của SQLite đóng gói chỉ khai thác được qua một file `-wal` do
    // kẻ tấn công chuẩn bị (D-032, D-075).
    let copy = scratch.join(DATABASE_FILE);
    std::fs::copy(directory.join(DATABASE_FILE), &copy)
        .map_err(|error| format!("cannot read {DATABASE_FILE} of the backup: {error}"))?;
    let source = open_sqlite(&copy, migrator).await.map_err(describe)?;
    // Xem CHÍNH bản sao (không phải database đang cấu hình): `copy_modules` chỉ chép module đang bật, dữ liệu của
    // module tắt sẽ bị bỏ lại trong im lặng.
    if let Err(error) = crate::modules::refuse_dormant_in(modules, &source, "restore").await {
        let _ = source.close().await;
        return Err(error);
    }
    Ok(source)
}

/// Bản backup do một bản Autoxide MỚI hơn làm (module chương trình này biết — bật hay tắt — mà có migration nó không
/// biết): từ chối. Bản backup trước khi có danh sách migration (trước 0.20) không có gì để kiểm.
pub(crate) fn refuse_newer_backup(manifest: &Value, migrator: &Migrator) -> Result<(), String> {
    let known: BTreeSet<String> = migrator.names().into_iter().collect();
    let modules = module_names(migrator);
    let newer: Vec<&str> = listed(manifest, "migrations")
        .filter(|name| !known.contains(*name))
        .filter(|name| modules.contains(module_of(name)))
        .collect();
    if newer.is_empty() {
        return Ok(());
    }
    let shown = newer.iter().take(3).copied().collect::<Vec<_>>().join(", ");
    Err(format!(
        "this backup holds {} migration(s) this Autoxide ({}) does not know ({shown}): it was made from a \
         database migrated by a newer release (`backup.json` names the version that wrote it); restore it with \
         that release",
        newer.len(),
        env!("CARGO_PKG_VERSION"),
    ))
}

/// Bản backup có dữ liệu của module không có trong chương trình này (module viết ngoài repo chưa ghép vào, hay đã bỏ):
/// từ chối, nêu tên. Module lạ không có dòng nào thì bỏ qua được.
fn refuse_unknown_modules(manifest: &Value, migrator: &Migrator) -> Result<(), String> {
    let modules = module_names(migrator);
    let strangers: BTreeSet<&str> = listed(manifest, "migrations")
        .map(module_of)
        .filter(|module| !modules.contains(*module))
        .collect();
    let mut with_rows = BTreeSet::new();
    for table in manifest["tables"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let (Some(name), Some(rows)) = (table["table"].as_str(), table["rows"].as_u64()) else {
            continue;
        };
        if rows == 0 {
            continue;
        }
        // Bảng của module `x` tên `x_…`; lấy module dài nhất khớp để `foo_bar_…` không bị nhận là của `foo`.
        let owner = strangers
            .iter()
            .copied()
            .chain(modules.iter().map(String::as_str))
            .filter(|module| name.starts_with(&format!("{module}_")))
            .max_by_key(|module| module.len());
        if let Some(owner) = owner.filter(|owner| strangers.contains(owner)) {
            with_rows.insert(owner);
        }
    }
    if with_rows.is_empty() {
        return Ok(());
    }
    Err(format!(
        "this backup holds data of modules this Autoxide does not have ({}); restore it with a build that \
         includes them",
        with_rows.into_iter().collect::<Vec<_>>().join(", ")
    ))
}

/// Số file thường trong `directory` (thư mục không có: 0). Cùng cách đi như `load_files`, symlink thì từ chối.
fn count_files(directory: &Path) -> Result<u64, String> {
    let mut files = 0;
    let mut pending = vec![directory.to_path_buf()];
    while let Some(folder) = pending.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", folder.display())),
        };
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() {
                return Err(format!(
                    "{} is a symbolic link; a backup made by `autoxide backup` has none",
                    entry.path().display()
                ));
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else {
                files += 1;
            }
        }
    }
    Ok(files)
}

/// Bản backup làm bằng `--all`: nhắc restore từng placement nó chứa.
pub(crate) fn placements_to_restore(manifest: &Value) -> String {
    let names: Vec<&str> = listed(manifest, "placements").collect();
    if names.is_empty() {
        return String::new();
    }
    format!(
        "The backup also holds these placements: {}. Restore each one into its own empty database with \
         `autoxide restore <directory> --placement <name>`.\n",
        names.join(", ")
    )
}

/// Các chuỗi trong mảng `field` của manifest.
pub(crate) fn listed<'a>(manifest: &'a Value, field: &str) -> impl Iterator<Item = &'a str> {
    manifest[field]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
}

/// Mọi module có migration trong chương trình này, bật hay tắt (`Migrator::names` có cả module tắt).
fn module_names(migrator: &Migrator) -> BTreeSet<String> {
    migrator.names().iter().map(|name| module_of(name).to_owned()).collect()
}

fn module_of(name: &str) -> &str {
    name.split('/').next().unwrap_or(name)
}
