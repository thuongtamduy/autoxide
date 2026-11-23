//! Migrated template database (D-20261120-test-db-template).
//!
//! End-to-end tests build every module, and previously each test reran every module's whole migration
//! on its own empty database — most of a test's time. Now migrations run ONCE into
//! a template database for each migration set, and each test then copies the template into its own database:
//!
//! - SQLite: copy the template file to the test's file location (a file that does not exist yet = nobody opened it).
//! - Postgres: `DROP DATABASE` the test's empty database then `CREATE DATABASE … TEMPLATE <template>` with
//!   the same name (the URL does not change). The template database is set to `ALLOW_CONNECTIONS false`:
//!   Postgres can only copy from a database with no connections.
//!
//! Template key: database kind, the migration list in order ([`Migrator::names`]), the running test
//! program (path, size, modification time — after a rebuild the key changes, so editing the content of an
//! uncommitted migration never reuses a stale template) and the extra part the caller supplies (e.g. the
//! program's secret key, for data upgrades that use it). Many tests and processes need the same
//! template: one builds, the others wait — a lock file (SQLite) or an advisory lock (Postgres).
//!
//! Copies only when the test's database is certainly untouched: for SQLite the file does not exist, for
//! Postgres there are no tables and nobody is connected. When unsure, or on any failure, it returns `false`
//! and the caller runs the migrations as before — the template only speeds things up, never changes the
//! result.
//!
//! `CREATE / DROP / ALTER DATABASE` and advisory locks are Postgres-only commands that the
//! query builder cannot express: hand-written SQL, only in test helpers (like `postgres.rs`, D-031).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use autoxide_kernel::config::{DatabaseBackend, DatabaseConfig, DatabaseUrl};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::Migrator;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use uuid::Uuid;

use crate::{TEST_DATABASE_URL, TestDatabaseError, postgres, sqlite};

/// Environment variable that turns the template database off (`TEST_DATABASE_TEMPLATE=off`): every test
/// migrates on an empty database as before — to measure, or to rule the template out when it is suspected of
/// causing a failure.
pub const TEST_DATABASE_TEMPLATE: &str = "TEST_DATABASE_TEMPLATE";

/// Prefix of template database names on Postgres: `autoxide_tpl_<16-hex key>_<UUIDv7>`.
pub(crate) const POSTGRES_PREFIX: &str = "autoxide_tpl_";

/// Directory holding SQLite template files (`<key>.sqlite`, `<key>.lock`) under the temp directory.
pub(crate) const SQLITE_DIRECTORY: &str = "autoxide-templates";

/// Whether the template database is used: not turned off by [`TEST_DATABASE_TEMPLATE`].
pub fn enabled() -> bool {
    !std::env::var(TEST_DATABASE_TEMPLATE)
        .is_ok_and(|value| matches!(value.trim(), "off" | "0" | "false" | "no"))
}

/// Brings the test database of `config` (created by [`TestDatabase`](crate::TestDatabase), not yet used)
/// to the state where every migration of `migrator` has run, by copying the template database.
///
/// Returns `true` when copied; `false` when not copied (turned off, the database is no longer untouched, it
/// is not a testkit database, or an error) — the caller still runs `migrator.up` as usual, in which case it
/// does all the work; after a copy, `up` has nothing left to do.
pub async fn prepare(config: &DatabaseConfig, migrator: &Migrator) -> bool {
    if !enabled() {
        return false;
    }
    let Ok(key) = key(config.url.backend(), migrator) else {
        return false;
    };
    match config.url.backend() {
        DatabaseBackend::Sqlite => sqlite_prepare(config, migrator, &key).await.unwrap_or(false),
        DatabaseBackend::Postgres => {
            postgres_prepare(config, migrator, &key).await.unwrap_or(false)
        }
    }
}

/// Key of a template database: 16 hex digits.
fn key(backend: DatabaseBackend, migrator: &Migrator) -> std::io::Result<String> {
    let executable = std::env::current_exe()?;
    let metadata = std::fs::metadata(&executable)?;
    // `DefaultHasher::new()` cho cùng kết quả ở mọi tiến trình của cùng một bản build — đủ, vì
    // khoá vốn gồm cả chương trình test.
    let mut hasher = DefaultHasher::new();
    format!("{backend:?}").hash(&mut hasher);
    migrator.names().hash(&mut hasher);
    executable.hash(&mut hasher);
    metadata.len().hash(&mut hasher);
    metadata.modified()?.hash(&mut hasher);
    migrator.uses_configured_key().hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

// --- SQLite -------------------------------------------------------------------------------------

async fn sqlite_prepare(
    config: &DatabaseConfig,
    migrator: &Migrator,
    key: &str,
) -> Result<bool, TestDatabaseError> {
    let Some(target) = config.url.sqlite_path() else {
        return Ok(false);
    };
    // Chỉ file của testkit (`<tạm>/autoxide-test-<UUIDv7>/test.sqlite`), và chỉ khi chưa ai mở
    // nó: SQLite tạo file ngay ở kết nối đầu tiên.
    let owned = target
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name.to_string_lossy().starts_with(sqlite::DIRECTORY_PREFIX));
    if !owned || target.exists() {
        return Ok(false);
    }
    let directory = std::env::temp_dir().join(SQLITE_DIRECTORY);
    std::fs::create_dir_all(&directory)?;
    let template = directory.join(format!("{key}.sqlite"));
    if !template.exists() {
        // Chờ lock trên thread riêng: tiến trình khác có thể đang dựng mẫu này (vài giây).
        let lock_path = directory.join(format!("{key}.lock"));
        let lock = tokio::task::spawn_blocking(move || -> std::io::Result<std::fs::File> {
            let file = std::fs::File::create(lock_path)?;
            file.lock()?;
            Ok(file)
        })
        .await
        .map_err(|error| TestDatabaseError(error.to_string()))??;
        // Bên giữ lock trước có thể đã dựng xong.
        if !template.exists() {
            sqlite_build(config, migrator, &directory, &template).await?;
        }
        // Đóng file thì nhả lock.
        drop(lock);
    }
    // Chép ra file tạm cạnh đích rồi đổi tên: bài test không bao giờ thấy file chép dở.
    let partial = target.with_extension(format!("partial-{}", Uuid::now_v7()));
    std::fs::copy(&template, &partial)?;
    std::fs::rename(&partial, &target)?;
    Ok(true)
}

/// Builds the template file: migrates a fresh file then renames it to `template` (a rename is atomic, so
/// the template file either does not exist or is complete).
async fn sqlite_build(
    config: &DatabaseConfig,
    migrator: &Migrator,
    directory: &Path,
    template: &Path,
) -> Result<(), TestDatabaseError> {
    let building = directory.join(format!("building-{}.sqlite", Uuid::now_v7()));
    let url = format!("sqlite://{}?mode=rwc", building.display());
    let built = migrate(config, &url, migrator).await;
    if built.is_ok() {
        std::fs::rename(&building, template)?;
    }
    // WAL đã gộp vào file chính khi đóng; xoá phần còn sót (và file dựng hỏng).
    for leftover in sqlite_companions(&building) {
        let _ = std::fs::remove_file(leftover);
    }
    built
}

fn sqlite_companions(file: &Path) -> [PathBuf; 3] {
    let with = |suffix: &str| PathBuf::from(format!("{}{suffix}", file.display()));
    [file.to_path_buf(), with("-wal"), with("-shm")]
}

/// Deletes SQLite template files older than [`STALE_AFTER`](crate::stale::STALE_AFTER) (by modification time:
/// a template file is never modified after it is built). A test using a template that was just deleted
/// rebuilds it.
pub(crate) fn remove_stale_sqlite(parent: &Path, now: SystemTime) -> usize {
    let Ok(entries) = std::fs::read_dir(parent.join(SQLITE_DIRECTORY)) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|metadata| metadata.modified()).is_ok_and(|modified| {
            now.duration_since(modified).is_ok_and(|age| age > crate::stale::STALE_AFTER)
        });
        if old && entry.path().is_file() && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

// --- Postgres -----------------------------------------------------------------------------------

async fn postgres_prepare(
    config: &DatabaseConfig,
    migrator: &Migrator,
    key: &str,
) -> Result<bool, TestDatabaseError> {
    let url = config.url.expose();
    let Some(name) = database_name(url) else {
        return Ok(false);
    };
    if !crate::stale::has_shape(&name, postgres::DATABASE_PREFIX) {
        return Ok(false);
    }
    // Kết nối quản trị: theo `TEST_DATABASE_URL` như `TestDatabase`; không có thì database
    // `postgres` trên cùng máy chủ.
    let admin_url = std::env::var(TEST_DATABASE_URL)
        .ok()
        .filter(|configured| DbBackend::Postgres.is_prefix_of(configured.trim()))
        .map(|configured| configured.trim().to_owned())
        .or_else(|| postgres::with_database(url, "postgres"))
        .ok_or_else(|| TestDatabaseError("cannot derive the admin URL".to_owned()))?;
    if !postgres_is_pristine(url).await? {
        return Ok(false);
    }
    let admin = postgres::connect_admin(&admin_url).await?;
    let result = postgres_clone(&admin, &admin_url, config, migrator, key, &name).await;
    admin.close().await?;
    result
}

/// A database with no tables, views or sequences other than the system's.
async fn postgres_is_pristine(url: &str) -> Result<bool, TestDatabaseError> {
    let connection = postgres::connect_admin(url).await?;
    let count = connection
        .query_one_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname NOT IN ('pg_catalog', 'information_schema') \
             AND n.nspname NOT LIKE 'pg\\_toast%'",
        ))
        .await;
    connection.close().await?;
    let count: i64 = match count? {
        Some(row) => row.try_get_by_index(0)?,
        None => return Ok(false),
    };
    Ok(count == 0)
}

async fn postgres_clone(
    admin: &DatabaseConnection,
    admin_url: &str,
    config: &DatabaseConfig,
    migrator: &Migrator,
    key: &str,
    name: &str,
) -> Result<bool, TestDatabaseError> {
    let template = match postgres_find(admin, key).await? {
        Some(template) => template,
        None => postgres_build(admin, admin_url, config, migrator, key).await?,
    };
    // Không FORCE: còn ai kết nối (bài test tự mở pool) thì lệnh hỏng và ta không chép.
    if admin.execute_unprepared(&format!("DROP DATABASE \"{name}\"")).await.is_err() {
        return Ok(false);
    }
    let cloned = admin
        .execute_unprepared(&format!("CREATE DATABASE \"{name}\" TEMPLATE \"{template}\""))
        .await;
    if cloned.is_err() {
        // Mẫu vừa bị dọn (tiến trình khác): trả lại database trống như cũ, bên gọi migrate.
        admin.execute_unprepared(&format!("CREATE DATABASE \"{name}\"")).await?;
        return Ok(false);
    }
    Ok(true)
}

/// The already built template of `key` (marked `IS_TEMPLATE`), if any.
async fn postgres_find(
    admin: &DatabaseConnection,
    key: &str,
) -> Result<Option<String>, TestDatabaseError> {
    let rows = admin
        .query_all_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT datname::text FROM pg_database WHERE datistemplate",
        ))
        .await?;
    let prefix = format!("{POSTGRES_PREFIX}{key}_");
    for row in rows {
        let name: String = row.try_get_by_index(0)?;
        if name.starts_with(&prefix) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

/// Builds the template of `key` under an advisory lock (one builds, the others wait then use it).
async fn postgres_build(
    admin: &DatabaseConnection,
    admin_url: &str,
    config: &DatabaseConfig,
    migrator: &Migrator,
    key: &str,
) -> Result<String, TestDatabaseError> {
    // Khoá 16 hex = 64 bit: đúng cỡ `bigint` của advisory lock.
    let lock =
        u64::from_str_radix(key, 16).map_err(|error| TestDatabaseError(error.to_string()))? as i64;
    admin.execute_unprepared(&format!("SELECT pg_advisory_lock({lock})")).await?;
    let built = async {
        if let Some(template) = postgres_find(admin, key).await? {
            return Ok(template);
        }
        let name = format!("{POSTGRES_PREFIX}{key}_{}", Uuid::now_v7().simple());
        admin.execute_unprepared(&format!("CREATE DATABASE \"{name}\"")).await?;
        let url = postgres::with_database(admin_url, &name)
            .ok_or_else(|| TestDatabaseError("cannot build the template URL".to_owned()))?;
        if let Err(error) = migrate(config, &url, migrator).await {
            let _ = postgres::drop_database(admin, &name).await;
            return Err(error);
        }
        // Đánh dấu xong sau cùng: `postgres_find` chỉ nhận mẫu đã đánh dấu.
        admin
            .execute_unprepared(&format!(
                "ALTER DATABASE \"{name}\" WITH ALLOW_CONNECTIONS false IS_TEMPLATE true"
            ))
            .await?;
        Ok(name)
    }
    .await;
    admin.execute_unprepared(&format!("SELECT pg_advisory_unlock({lock})")).await?;
    built
}

/// Deletes template databases older than [`STALE_AFTER`](crate::stale::STALE_AFTER) (the build time is in the
/// UUIDv7 at the end of the name). A test using a template that was just deleted rebuilds it. Returns the
/// number of templates deleted.
pub(crate) async fn remove_stale_postgres(
    admin: &DatabaseConnection,
    now: SystemTime,
) -> Result<usize, TestDatabaseError> {
    let rows = admin
        .query_all_raw(Statement::from_string(
            DbBackend::Postgres,
            "SELECT datname::text FROM pg_database",
        ))
        .await?;
    let mut removed = 0;
    for row in rows {
        let name: String = row.try_get_by_index(0)?;
        let Some(rest) = name.strip_prefix(POSTGRES_PREFIX) else {
            continue;
        };
        // `<khoá 16 hex>_<UUIDv7 32 hex>`.
        let Some((_, uuid)) = rest.split_once('_') else {
            continue;
        };
        if !crate::stale::is_stale(uuid, "", now) {
            continue;
        }
        // Mẫu phải bỏ đánh dấu trước khi xoá được; lỗi (vd bên khác đang chép) thì bỏ qua.
        let unmark = format!("ALTER DATABASE \"{name}\" WITH IS_TEMPLATE false");
        if admin.execute_unprepared(&unmark).await.is_ok()
            && postgres::drop_database(admin, &name).await.is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Database name in a URL `scheme://authority/<name>?…`.
fn database_name(url: &str) -> Option<String> {
    let base = url.split('?').next()?;
    let (_, rest) = base.split_once("://")?;
    let (_, name) = rest.split_once('/')?;
    (!name.is_empty()).then(|| name.to_owned())
}

// --- chung --------------------------------------------------------------------------------------

/// Runs every migration on the database at `url`, with the same connection settings as a test (the kernel sets
/// WAL… itself for SQLite), then closes all connections.
async fn migrate(
    config: &DatabaseConfig,
    url: &str,
    migrator: &Migrator,
) -> Result<(), TestDatabaseError> {
    let mut config = config.clone();
    config.url =
        url.parse::<DatabaseUrl>().map_err(|error| TestDatabaseError(error.to_string()))?;
    config.read_url = None;
    let database =
        Database::connect(&config).await.map_err(|error| TestDatabaseError(error.to_string()))?;
    let migrated = migrator.up(&database).await.map(|_| ());
    database.close().await?;
    migrated.map_err(|error| TestDatabaseError(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lay_ten_database_trong_url() {
        assert_eq!(database_name("postgres://u:p@h:1/abc?x=1").as_deref(), Some("abc"));
        assert_eq!(database_name("postgres://h/abc").as_deref(), Some("abc"));
        assert_eq!(database_name("postgres://h"), None);
        assert_eq!(database_name("postgres://h/"), None);
    }
}
