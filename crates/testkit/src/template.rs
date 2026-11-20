//! Database mẫu đã migrate (D-20261120-test-db-template).
//!
//! Test đầu-cuối dựng mọi module, và trước đây mỗi bài chạy lại toàn bộ migration của mọi module
//! trên database trống của nó — phần lớn thời gian của bài test. Giờ migration chạy MỘT lần vào
//! một database mẫu cho mỗi bộ migration, rồi mỗi bài test chép database mẫu vào database của nó:
//!
//! - SQLite: chép file mẫu vào chỗ file của bài test (file chưa tồn tại = chưa ai mở).
//! - Postgres: `DROP DATABASE` database trống của bài test rồi `CREATE DATABASE … TEMPLATE <mẫu>`
//!   cùng tên (URL không đổi). Database mẫu đặt `ALLOW_CONNECTIONS false`: Postgres chỉ chép được
//!   từ database không có kết nối nào.
//!
//! Khoá của mẫu: loại database, danh sách migration theo thứ tự ([`Migrator::names`]), chương
//! trình test đang chạy (đường dẫn, kích thước, thời điểm sửa — build lại thì khoá đổi, nên sửa nội
//! dung một migration chưa commit cũng không dùng nhầm mẫu cũ) và phần thêm bên gọi đưa vào (vd
//! khoá bí mật của chương trình, upgrade dữ liệu dùng nó). Nhiều test, nhiều tiến trình cùng cần
//! một mẫu: một bên dựng, bên khác chờ — lock file (SQLite) hay advisory lock (Postgres).
//!
//! Chỉ chép khi chắc database của bài test còn nguyên: SQLite thì file chưa có, Postgres thì không
//! có bảng nào và không ai đang kết nối. Không chắc, hay có gì hỏng, thì trả `false` và bên gọi chạy
//! migration như cũ — mẫu chỉ làm nhanh, không bao giờ đổi kết quả.
//!
//! `CREATE / DROP / ALTER DATABASE` và advisory lock là lệnh chỉ Postgres có, không viết được bằng
//! query builder: SQL viết tay, chỉ trong đồ test (như `postgres.rs`, D-031).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use autoxide_kernel::config::{DatabaseBackend, DatabaseConfig, DatabaseUrl};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::Migrator;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use uuid::Uuid;

use crate::{TEST_DATABASE_URL, TestDatabaseError, postgres, sqlite};

/// Biến môi trường tắt database mẫu (`TEST_DATABASE_TEMPLATE=off`): mọi test migrate trên database
/// trống như trước — để đo, hay để loại trừ khi nghi mẫu gây lỗi.
pub const TEST_DATABASE_TEMPLATE: &str = "TEST_DATABASE_TEMPLATE";

/// Tiền tố tên database mẫu trên Postgres: `autoxide_tpl_<khoá 16 hex>_<UUIDv7>`.
pub(crate) const POSTGRES_PREFIX: &str = "autoxide_tpl_";

/// Thư mục chứa file mẫu SQLite (`<khoá>.sqlite`, `<khoá>.lock`) dưới thư mục tạm.
pub(crate) const SQLITE_DIRECTORY: &str = "autoxide-templates";

/// Database mẫu có được dùng không: không tắt bằng [`TEST_DATABASE_TEMPLATE`].
pub fn enabled() -> bool {
    !std::env::var(TEST_DATABASE_TEMPLATE)
        .is_ok_and(|value| matches!(value.trim(), "off" | "0" | "false" | "no"))
}

/// Đưa database test của `config` (do [`TestDatabase`](crate::TestDatabase) tạo, chưa ai dùng)
/// về trạng thái đã chạy mọi migration của `migrator`, bằng cách chép database mẫu.
///
/// Trả `true` khi đã chép; `false` khi không chép (đã tắt, database không còn nguyên, không phải
/// database của testkit, hay lỗi) — bên gọi vẫn chạy `migrator.up` như thường, lúc đó nó làm hết
/// việc; sau khi chép thì `up` không còn gì để làm.
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

/// Khoá của database mẫu: 16 chữ số hex.
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

/// Dựng file mẫu: migrate một file mới rồi đổi tên thành `template` (đổi tên là nguyên tử, nên
/// file mẫu luôn hoặc chưa có hoặc đầy đủ).
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

/// Xoá file mẫu SQLite cũ hơn [`STALE_AFTER`](crate::stale::STALE_AFTER) (theo thời điểm sửa:
/// file mẫu không bao giờ bị sửa sau khi dựng). Bài test đang dùng mẫu vừa bị xoá thì dựng lại.
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

/// Database chưa có bảng, view hay sequence nào ngoài của hệ thống.
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

/// Mẫu đã dựng xong của `key` (đã đánh dấu `IS_TEMPLATE`), nếu có.
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

/// Dựng mẫu của `key` dưới advisory lock (một bên dựng, bên khác chờ rồi dùng).
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

/// Xoá database mẫu cũ hơn [`STALE_AFTER`](crate::stale::STALE_AFTER) (thời điểm dựng nằm trong
/// UUIDv7 cuối tên). Bài test đang dùng mẫu vừa bị xoá thì dựng lại. Trả về số mẫu đã xoá.
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

/// Tên database trong URL `scheme://authority/<tên>?…`.
fn database_name(url: &str) -> Option<String> {
    let base = url.split('?').next()?;
    let (_, rest) = base.split_once("://")?;
    let (_, name) = rest.split_once('/')?;
    (!name.is_empty()).then(|| name.to_owned())
}

// --- chung --------------------------------------------------------------------------------------

/// Chạy mọi migration lên database ở `url`, cùng cấu hình kết nối như bài test (kernel tự đặt
/// WAL… cho SQLite), rồi đóng hết kết nối.
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
