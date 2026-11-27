//! `autoxide backup --all` và `autoxide restore --placement <tên>` (bước 71 G3, D-20261126-upgrade-rollback): database
//! riêng của từng khách (placement) và nơi lưu file riêng của nó, cạnh bản backup của database chính:
//!
//! ```text
//! <thư mục>/placements/<tên>/backup.json, database.sqlite, files/…
//! ```
//!
//! Mỗi placement một bản backup cùng định dạng, giữ đúng schema của placement đó lúc backup. Restore vào một
//! placement có database trống (thêm bằng `autoxide placement add`), file vào nơi lưu riêng của nó hay nơi lưu chung.

use std::path::{Path, PathBuf};

use autoxide_kernel::backup::{
    self, BackupOf, BackupPlacement, DATABASE_FILE, FILES_DIR, open_sqlite, private_dir,
};
use autoxide_kernel::storage::Storage;
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, PlacementId, TenancyMode};
use autoxide_kernel::transfer::copy_modules;

use super::backup::{describe, load_files, read_manifest, summary};
use crate::runtime::{self, Migrations, Runtime};
use crate::settings::Settings;

/// Thư mục con chứa bản backup của các placement.
pub(crate) const PLACEMENTS_DIR: &str = "placements";

/// Backup mọi placement (trừ database chính) vào `<directory>/placements/<tên>/`; trả nội dung in ra và tên các placement
/// đã backup (ghi vào manifest của database chính, viết sau cùng).
///
/// Đọc sổ placement của database chính: database chính đi sau chương trình này quá xa (sổ có cột chương trình chưa
/// biết hay thiếu cột) thì báo rõ. Đừng chuyển / tách / gộp khách trong lúc chạy: một khách chuyển giữa hai placement
/// có thể có mặt ở cả hai bản hay ở không bản nào.
pub(crate) async fn write_all(
    runtime: &Runtime,
    modules: &[&str],
    directory: &Path,
) -> Result<(String, Vec<String>), String> {
    let Some(registry) = runtime.modules.tenancy() else { return Ok((String::new(), Vec::new())) };
    if registry.mode() != TenancyMode::Multi {
        return Ok((String::new(), Vec::new()));
    }
    let placements = runtime.modules.placements(&registry).map_err(|error| error.to_string())?;
    let listed = placements.list().await.map_err(|error| {
        format!(
            "cannot read the placement register of the main database ({}); back up with the release the \
             main database runs, or migrate the main database first",
            describe(error)
        )
    })?;
    // Data upgrade trên bản nháp của placement dùng khoá của database chính, như khi migrate placement.
    let keyed =
        runtime.modules.migrator().map_err(|error| error.to_string())?.keyed_on(&runtime.database);
    let mut text = String::new();
    let mut names = Vec::new();
    for found in listed {
        if found.id == MAIN_PLACEMENT.as_uuid() {
            continue;
        }
        let target = placement_directory(directory, &found.name)?;
        private_dir(&target).map_err(|error| format!("{}: {error}", target.display()))?;
        let id = PlacementId::from_uuid(found.id);
        let database = registry.placement_database(id).await.map_err(describe)?;
        // Nơi lưu riêng của placement; không có thì file của nó nằm ở nơi lưu chung, đã có trong bản chính.
        let files = registry.placement_storage(id).await.map_err(describe)?;
        let scratch =
            std::env::temp_dir().join(format!("autoxide-backup-{}", uuid::Uuid::now_v7()));
        let manifest = backup::write_backup_of(
            &database,
            &keyed,
            modules,
            files,
            &backup::DirectorySink::new(&target),
            &scratch,
            BackupOf::placement(BackupPlacement::new(found.id, found.name.clone())),
        )
        .await
        .map_err(|error| format!("placement `{}`: {}", found.name, describe(error)))?;
        text.push_str(&format!(
            "Backed up placement {}: {} rows in {} tables and {} files.\n",
            found.name,
            manifest.rows(),
            manifest.tables.len(),
            manifest.files
        ));
        names.push(found.name);
    }
    Ok((text, names))
}

/// `<directory>/placements/<name>`; tên placement chỉ gồm chữ thường, số, `-` và `_` (không đi ra ngoài thư mục).
fn placement_directory(directory: &Path, name: &str) -> Result<PathBuf, String> {
    let plain =
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !plain {
        return Err(format!("placement name `{name}` cannot be used as a directory name"));
    }
    Ok(directory.join(PLACEMENTS_DIR).join(name))
}

/// Đưa bản backup của một placement (`<thư mục>/placements/<tên>/`) vào CHÍNH placement đó (cùng tên, cùng id), database
/// phải trống. Phục hồi sau sự cố: tạo lại database trống ở đúng URL của placement, rồi restore vào đúng tên — không
/// `placement add` một placement mới (khách vẫn trỏ tới id cũ).
pub(crate) async fn restore_placement(
    settings: Settings,
    directory: PathBuf,
    name: String,
) -> Result<String, String> {
    let manifest = read_manifest(&directory).await?;
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let result = restore_into(&runtime, &directory, &name, &manifest).await;
    let _ = runtime.database.close().await;
    result
}

async fn restore_into(
    runtime: &Runtime,
    directory: &Path,
    name: &str,
    manifest: &serde_json::Value,
) -> Result<String, String> {
    let registry =
        runtime.modules.tenancy().ok_or("the modules were not started with a tenancy registry")?;
    let placements = runtime.modules.placements(&registry).map_err(|error| error.to_string())?;
    let found = placements
        .list()
        .await
        .map_err(describe)?
        .into_iter()
        .find(|found| found.name == name && found.id != MAIN_PLACEMENT.as_uuid())
        .ok_or_else(|| {
            format!("there is no placement `{name}`; add it first (`autoxide placement add`)")
        })?;
    let meant = &manifest["placement"];
    if meant["name"].as_str() != Some(name)
        || meant["id"].as_str() != Some(found.id.to_string().as_str())
    {
        let what = match meant["name"].as_str() {
            Some(other) => format!("the backup of placement `{other}`"),
            None => "the backup of the main database".to_owned(),
        };
        return Err(format!(
            "this is {what}, not of placement `{name}` (id {}); restore a placement's backup into that same \
             placement",
            found.id
        ));
    }
    let id = PlacementId::from_uuid(found.id);
    let target = registry.placement_database(id).await.map_err(describe)?;
    let migrator =
        runtime.modules.migrator().map_err(|error| error.to_string())?.keyed_on(&runtime.database);
    let states = migrator.status(&target).await.map_err(|error| error.to_string())?;
    if states.iter().any(|state| state.applied) {
        return Err(format!(
            "the database of placement `{name}` already has Autoxide tables; restore into a placement with a new, \
             empty database"
        ));
    }
    migrator.up(&target).await.map_err(|error| error.to_string())?;

    // Như restore của database chính: làm trên bản sao của file backup, chỉ chép đúng `database.sqlite`.
    let scratch = std::env::temp_dir().join(format!("autoxide-restore-{}", uuid::Uuid::now_v7()));
    private_dir(&scratch).map_err(|error| error.to_string())?;
    let copied = async {
        let copy = scratch.join(DATABASE_FILE);
        std::fs::copy(directory.join(DATABASE_FILE), &copy)
            .map_err(|error| format!("cannot read {DATABASE_FILE} of the backup: {error}"))?;
        let source = open_sqlite(&copy, &migrator).await.map_err(describe)?;
        let modules = runtime.modules.names();
        // Bản backup có dữ liệu của module mà bản cài này đang tắt: dừng, không bỏ sót nó.
        let tables =
            match crate::modules::refuse_dormant_data(&runtime.modules, &source, "backup restore")
                .await
            {
                Ok(()) => copy_modules(&source, &target, &modules)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            };
        let _ = source.close().await;
        tables
    }
    .await;
    let _ = std::fs::remove_dir_all(&scratch);
    let tables = copied?;
    let storage = match registry.placement_storage(id).await.map_err(describe)? {
        Some(own) => Some(own),
        None => runtime.modules.api::<dyn Storage>(),
    };
    let files = match storage {
        Some(storage) => load_files(storage, &directory.join(FILES_DIR)).await?,
        None => 0,
    };
    let expected = manifest["files"].as_u64().unwrap_or_default();
    if files != expected {
        return Err(format!(
            "the backup lists {expected} files but {files} were found in its `{FILES_DIR}` directory"
        ));
    }
    // Placement đủ migration: khách trên đó phục vụ lại được.
    placements.check().await.map_err(describe)?;
    Ok(summary(&format!("Restored placement {name}:"), &tables, files, directory))
}
