//! `autoxide backup --all` và `autoxide restore --placement <tên>` (bước 71 G3, D-20261126-upgrade-rollback): database
//! riêng của từng khách (placement) và nơi lưu file riêng của nó, cạnh bản backup của database chính:
//!
//! ```text
//! <thư mục>/placements/<tên>/backup.json, database.sqlite, files/…
//! ```
//!
//! Mỗi placement một bản backup cùng định dạng, giữ đúng schema của placement đó lúc backup. Restore vào CHÍNH
//! placement đó (cùng tên, cùng id) khi database của nó trống lại, file vào nơi lưu riêng của nó hay nơi lưu chung.

use std::path::{Path, PathBuf};

use autoxide_kernel::backup::{self, BackupOf, BackupPlacement, FILES_DIR, private_dir};
use autoxide_kernel::migration::MigrationLock;
use autoxide_kernel::storage::Storage;
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, PlacementId, Placements, TenancyMode};
use autoxide_kernel::transfer::{CopiedTable, copy_modules};

use super::backup::{
    describe, load_files, migrations_behind, read_manifest, refuse_non_empty, summary,
};
use super::backup_checks::open_checked;
use crate::runtime::{self, Migrations, Runtime};
use crate::settings::Settings;
use crate::startup::LOCK_WAIT;

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
        let behind = migrations_behind(&keyed, &database)
            .await
            .map_err(|error| format!("placement `{}`: {error}", found.name))?;
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
        let note = match behind {
            0 => String::new(),
            count => format!(" (its database is {count} migration(s) behind this program)"),
        };
        text.push_str(&format!(
            "Backed up placement {}: {} rows in {} tables and {} files{note}.\n",
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
    let (directory, manifest) = placement_backup(directory, &name).await?;
    let runtime = runtime::start(settings, Migrations::RequireApplied).await?;
    let result = restore_into(&runtime, &directory, &name, &manifest).await;
    let _ = runtime.database.close().await;
    result
}

/// Bản backup của placement `name`: `directory` là chính nó (`<thư mục>/placements/<tên>`), hay bản backup của database
/// chính làm bằng `--all` — khi ấy lấy `placements/<tên>/` của nó, và từ chối rõ khi bản đó không chứa placement này.
async fn placement_backup(
    directory: PathBuf,
    name: &str,
) -> Result<(PathBuf, serde_json::Value), String> {
    let manifest = read_manifest(&directory).await?;
    if !manifest["placement"].is_null() {
        return Ok((directory, manifest));
    }
    let held: Vec<&str> = manifest["placements"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    if !held.contains(&name) {
        let what = if held.is_empty() {
            "no placement (it was made without `--all`)".to_owned()
        } else {
            format!("only these placements: {}", held.join(", "))
        };
        return Err(format!(
            "this is the backup of the main database and it holds {what}, not placement `{name}`; \
             give the backup directory of `{name}`"
        ));
    }
    let inner = placement_directory(&directory, name)?;
    let manifest = read_manifest(&inner).await?;
    Ok((inner, manifest))
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
    refuse_non_empty(&migrator, &target, &format!("the database of placement `{name}`")).await?;

    // Mọi kiểm trên bản sao trước, đích chưa bị chạm (như restore của database chính).
    let scratch = std::env::temp_dir().join(format!("autoxide-restore-{}", uuid::Uuid::now_v7()));
    private_dir(&scratch).map_err(|error| error.to_string())?;
    let copied = async {
        let source =
            open_checked(&runtime.modules, &migrator, manifest, directory, &scratch).await?;
        // Giữ lock migration và đặt placement `migrating` trong lúc chép (database lẫn file): khách của nó nhận 503
        // thay vì đọc/ghi một placement đang dở (máy chủ có thể vẫn chạy), và không ai migrate nó cùng lúc.
        let storage = match registry.placement_storage(id).await.map_err(describe)? {
            Some(own) => Some(own),
            None => runtime.modules.api::<dyn Storage>(),
        };
        let restore = Restore { name, migrator: &migrator, source: &source, target: &target };
        let copied = match placements.lock(LOCK_WAIT).await.map_err(describe) {
            Ok(held) => {
                let copied = restore.run(runtime, &placements, &held, storage, directory).await;
                if let Err(error) = held.release().await {
                    tracing::warn!(error = %error, "cannot release the migration lock");
                }
                copied
            }
            Err(error) => Err(error),
        };
        let _ = source.close().await;
        copied
    }
    .await;
    let _ = std::fs::remove_dir_all(&scratch);
    let (tables, files) = copied?;
    // Placement đủ migration: khách trên đó phục vụ lại được.
    placements.check().await.map_err(describe)?;
    Ok(summary(&format!("Restored placement {name}:"), &tables, files, directory))
}

/// Một lần restore placement đã qua kiểm, chạy khi giữ lock migration.
struct Restore<'a> {
    name: &'a str,
    migrator: &'a autoxide_kernel::migration::Migrator,
    source: &'a autoxide_kernel::database::Database,
    target: &'a autoxide_kernel::database::Database,
}

impl Restore<'_> {
    /// Kiểm lại database đích còn trống (lúc này đã giữ lock), đặt placement `migrating`, migrate nó, chép dữ liệu và
    /// file, rồi đặt `behind` (`check` sau đó đặt `ready`). Lỗi sau khi đã migrate đích: đưa đích về trống (gỡ mọi
    /// migration) — placement còn thiếu migration nên giữ `behind`, khách nhận 503, không bao giờ được phục vụ một
    /// database dở; chạy lại lệnh là đủ. Gỡ cũng lỗi: để `migrating` (không ai tự đưa nó về `ready`) và báo cả hai.
    async fn run(
        &self,
        runtime: &Runtime,
        placements: &Placements,
        held: &MigrationLock,
        storage: Option<std::sync::Arc<dyn Storage>>,
        directory: &Path,
    ) -> Result<(Vec<CopiedTable>, u64), String> {
        let name = self.name;
        // Kiểm lại khi đã giữ lock (lần kiểm trước lock chỉ để báo sớm), trước khi đổi trạng thái: placement đang phục
        // vụ thì không bị đặt `migrating` oan.
        let what = format!("the database of placement `{name}`");
        refuse_non_empty(self.migrator, self.target, &what).await?;
        placements.mark_rollback(held, name, false).await.map_err(describe)?;
        let restored = async {
            self.migrator.up(self.target).await.map_err(|error| error.to_string())?;
            let tables = copy_modules(self.source, self.target, &runtime.modules.names())
                .await
                .map_err(|error| error.to_string())?;
            let files = match storage {
                Some(storage) => load_files(storage, &directory.join(FILES_DIR)).await?,
                None => 0,
            };
            Ok::<_, String>((tables, files))
        }
        .await;
        match restored {
            Ok(done) => match placements.mark_rollback(held, name, true).await {
                Ok(()) => Ok(done),
                Err(error) => Err(describe(error)),
            },
            Err(error) => match self.migrator.rollback(self.target, &|_, _| false).await {
                Ok(_) => Err(finish(
                    placements,
                    held,
                    name,
                    format!(
                        "{error}. The database of placement `{name}` was emptied again; fix the cause and run \
                         the command again"
                    ),
                )
                .await),
                Err(undo) => Err(format!(
                    "{error}. Emptying the database of placement `{name}` again failed too ({undo}); it stays \
                     `migrating` (its tenants get 503): drop and recreate that database, then run the command again"
                )),
            },
        }
    }
}

/// Đặt placement `behind` sau một lần restore không thành; lỗi khi đặt thì báo kèm, không che lỗi gốc.
async fn finish(
    placements: &Placements,
    held: &MigrationLock,
    name: &str,
    error: String,
) -> String {
    match placements.mark_rollback(held, name, true).await {
        Ok(()) => error,
        Err(mark) => {
            format!("{error} (and cannot mark placement `{name}` `behind`: {})", describe(mark))
        }
    }
}
