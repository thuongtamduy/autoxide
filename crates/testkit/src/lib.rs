//! Test helpers for Autoxide.
//!
//! The main piece is [`TestDatabase`]: one empty database per test, on
//! SQLite or Postgres depending on the `TEST_DATABASE_URL` environment variable. This way the same
//! test runs on both databases without changes (rule number one
//! in `CLAUDE.md`), and tests running in parallel never trample each other's data.
//!
//! ```no_run
//! use autoxide_testkit::TestDatabase;
//!
//! #[tokio::test]
//! async fn ten_bai_test() {
//!     TestDatabase::run(async |database| {
//!         let connection = database.connect().await.unwrap();
//!     // ... use `connection` ...
//!     })
//!     .await
//!     .unwrap();
//! }
//! ```
//!
//! Why `run(...)` wraps the test instead of a guard that cleans up when it leaves
//! scope (`Drop`): deleting the database is `async` work, and `Drop` cannot `await`.
//! Waiting inside `Drop` blocks a tokio runtime thread, while Postgres's
//! `DROP DATABASE` waits for every connection still mid-handshake — served by that very
//! blocked thread — so the two wait on each other for up to 60 seconds (observed and measured
//! on 11/11/2026). Handing the work to a background thread lets the test process exit before
//! it finishes deleting, leaving the database behind. `run` can `await` the deletion: it is
//! certain to finish and blocks nobody.
//!
//! [`TestApp`] does not migrate every database from scratch: it copies a migrated template database (module
//! `template`, D-20261120-test-db-template). The database is still private to each test.
//!
//! This crate may only be declared in `[dev-dependencies]` (checked by `cargo xtask check`).

pub mod ai;
mod app;
mod contract;
mod coverage;
mod dns;
mod einvoice;
mod identity;
mod leak;
mod mail;
pub mod mcp;
mod ownership;
mod payment;
mod postgres;
mod provider;
mod push;
mod s3;
mod schema;
mod sqlite;
mod stale;
mod statements;
mod storage;
mod store;
mod template;
mod tenancy;
mod voice;

pub use app::{TestApp, TestAppBuilder, TestAppError, TestRequest, TestResponse, tool};
pub use coverage::{TEST_API_COVERAGE, record_response};
pub use dns::FakeDns;
pub use einvoice::{FAKE_EINVOICE_PROVIDER, FakeEInvoiceProvider};
pub use identity::{FAKE_IDENTITY_PROVIDER, FakeIdentityVerifier, FakeSmsSender};
pub use leak::{branch_leak_check, cursor_leak_check, leak_check};
pub use mail::FakeMailer;
pub use ownership::schema_ownership_violations;
pub use payment::{
    FAKE_PROVIDER, FAKE_RATE_PROVIDER, FakeBankFeed, FakeExchangeRateSource, FakePaymentGateway,
    FakePayoutGateway, FakePayoutOutcome,
};
pub use provider::{FakeProviderServer, ProviderRequest};
pub use push::{FakeMobilePushSender, FakeWebPushSender, FakeZnsSender};
pub use s3::{
    TEST_S3_ACCESS_KEY_ID, TEST_S3_BUCKET, TEST_S3_ENDPOINT, TEST_S3_SECRET_ACCESS_KEY, s3_settings,
};
pub use schema::{ColumnSnapshot, IndexSnapshot, SchemaSnapshot, TableSnapshot, schema_snapshot};
pub use storage::{MemoryStorage, MemoryStorageOpener};
pub use store::FakeStorePurchases;
pub use template::{
    TEST_DATABASE_TEMPLATE, enabled as template_enabled, prepare as prepare_from_template,
};
pub use tenancy::{FakeTenantSettings, schema_tenancy_violations};

/// Handles across every tenant of `database`, for a test that reads or writes tables directly (look at rows
/// a service wrote, put an old-format row back before a migration): `all_tenants(&db).read().sql(…)`.
/// Tests only — services go through `tenant.db()`.
pub fn all_tenants(
    database: &autoxide_kernel::database::Database,
) -> autoxide_kernel::db::AllTenants<'_> {
    autoxide_kernel::db::all_tenants_for_tests(database)
}
pub use voice::{FAKE_RECORDING, FAKE_VOICE_PROVIDER, FakeVoiceProvider};

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbBackend, DbErr};

/// Name of the environment variable that selects the database for tests.
///
/// - Unset: SQLite, one file per test in a temporary directory.
/// - `postgres://user:password@host:port/<any database>`: Postgres. The user
///   in the URL needs the `CREATE DATABASE` permission; each test creates its own database.
pub const TEST_DATABASE_URL: &str = "TEST_DATABASE_URL";

/// Error while building a test database.
#[derive(Debug)]
pub struct TestDatabaseError(String);

impl fmt::Display for TestDatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "test database: {}", self.0)
    }
}

impl std::error::Error for TestDatabaseError {}

// `From` cho phép viết `?` trên kết quả của SeaORM trong hàm trả `TestDatabaseError`:
// Rust tự gọi `from` để đổi kiểu lỗi.
impl From<DbErr> for TestDatabaseError {
    fn from(error: DbErr) -> Self {
        TestDatabaseError(error.to_string())
    }
}

impl From<std::io::Error> for TestDatabaseError {
    fn from(error: std::io::Error) -> Self {
        TestDatabaseError(error.to_string())
    }
}

/// Where test databases are created, read from [`TEST_DATABASE_URL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target<'a> {
    Sqlite,
    /// With the admin URL (the URL the user set in the environment variable).
    Postgres(&'a str),
}

impl<'a> Target<'a> {
    /// `None` or an empty string means SQLite.
    fn parse(configured_url: Option<&'a str>) -> Result<Self, TestDatabaseError> {
        match configured_url.map(str::trim).filter(|url| !url.is_empty()) {
            None => Ok(Target::Sqlite),
            Some(url) if DbBackend::Sqlite.is_prefix_of(url) => Ok(Target::Sqlite),
            Some(url) if DbBackend::Postgres.is_prefix_of(url) => Ok(Target::Postgres(url)),
            Some(url) => Err(TestDatabaseError(format!(
                "{TEST_DATABASE_URL} must be unset (SQLite) or a postgres:// URL, got `{}`",
                postgres::redact_password(url)
            ))),
        }
    }
}

/// An empty database reserved for one test. Obtained through [`TestDatabase::run`].
#[derive(Debug)]
pub struct TestDatabase {
    url: String,
    backend: DbBackend,
    cleanup: Cleanup,
    /// Extra databases of the same test ([`TestDatabase::extra_database`]), cleaned up together.
    extras: std::sync::Mutex<Vec<Cleanup>>,
}

/// Work to clean up after the test has run.
#[derive(Debug)]
enum Cleanup {
    /// Deletes the temporary directory holding the SQLite file (with the `-wal` and `-shm` files).
    SqliteDirectory(PathBuf),
    /// `DROP DATABASE` over the admin connection.
    PostgresDatabase { admin_url: String, name: String },
}

/// Whether leftovers from earlier runs have been cleaned in this process.
static STALE_REMOVED: AtomicBool = AtomicBool::new(false);

impl TestDatabase {
    /// Creates an empty database according to the [`TEST_DATABASE_URL`] environment variable, runs
    /// `test` with it, deletes the database, then returns the result of `test`.
    ///
    /// `test` is an async closure: `async |database| { ... }`. If `test` panics
    /// (the test fails) the database is NOT deleted immediately; a later test run cleans it up
    /// once it is old enough (see module `stale`).
    //
    // `AsyncFnOnce(&TestDatabase) -> R`: closure async, gọi đúng một lần, mượn
    // (`&`) database chứ không giữ luôn — nhờ đó `run` còn quyền xoá nó sau khi
    // closure chạy xong.
    pub async fn run<R>(
        test: impl AsyncFnOnce(&TestDatabase) -> R,
    ) -> Result<R, TestDatabaseError> {
        // `var` trả `Err` khi biến không tồn tại; `.ok()` đổi thành `None`.
        let configured = std::env::var(TEST_DATABASE_URL).ok();
        Self::run_for(configured.as_deref(), test).await
    }

    /// Like [`run`](Self::run) but takes the URL directly instead of reading the environment.
    /// `None` or an empty string means SQLite. Use when a test needs a database
    /// of a specific kind.
    pub async fn run_for<R>(
        configured_url: Option<&str>,
        test: impl AsyncFnOnce(&TestDatabase) -> R,
    ) -> Result<R, TestDatabaseError> {
        let database = Self::create(Target::parse(configured_url)?).await?;
        let result = test(&database).await;
        database.destroy().await?;
        Ok(result)
    }

    async fn create(target: Target<'_>) -> Result<Self, TestDatabaseError> {
        // Lần đầu trong mỗi tiến trình test: dọn database mà một lần chạy đã chết
        // để sót. `swap` trả giá trị CŨ, nên chỉ lần gọi đầu tiên thấy `false`.
        // Dọn hỏng thì thôi — không vì thế mà làm đỏ bài test.
        if !STALE_REMOVED.swap(true, Ordering::Relaxed) {
            let _ = remove_stale(target, &std::env::temp_dir(), SystemTime::now()).await;
        }

        match target {
            Target::Sqlite => {
                let (directory, url) = sqlite::create_directory(&std::env::temp_dir())?;
                Ok(TestDatabase {
                    url,
                    backend: DbBackend::Sqlite,
                    cleanup: Cleanup::SqliteDirectory(directory),
                    extras: std::sync::Mutex::default(),
                })
            }
            Target::Postgres(admin_url) => {
                let admin = postgres::connect_admin(admin_url).await?;
                let created = postgres::create_database(&admin, uuid::Uuid::now_v7()).await;
                admin.close().await?;
                let name = created?;
                let url = postgres::with_database(admin_url, &name).ok_or_else(|| {
                    TestDatabaseError(format!(
                        "cannot find the host part in {TEST_DATABASE_URL} `{}`",
                        postgres::redact_password(admin_url)
                    ))
                })?;
                Ok(TestDatabase {
                    url,
                    backend: DbBackend::Postgres,
                    cleanup: Cleanup::PostgresDatabase { admin_url: admin_url.to_owned(), name },
                    extras: std::sync::Mutex::default(),
                })
            }
        }
    }

    /// Deletes the database. Takes `self` (not `&self`): after this call the value
    /// can no longer be used — the compiler rejects using a deleted database.
    async fn destroy(self) -> Result<(), TestDatabaseError> {
        // Mutex hỏng (một luồng panic khi đang giữ) vẫn lấy được dữ liệu bên trong.
        let extras = self.extras.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
        for extra in extras {
            Self::remove(extra).await?;
        }
        Self::remove(self.cleanup).await
    }

    async fn remove(cleanup: Cleanup) -> Result<(), TestDatabaseError> {
        match cleanup {
            Cleanup::SqliteDirectory(directory) => Ok(std::fs::remove_dir_all(directory)?),
            Cleanup::PostgresDatabase { admin_url, name } => {
                let admin = postgres::connect_admin(&admin_url).await?;
                let dropped = postgres::drop_database(&admin, &name).await;
                admin.close().await?;
                Ok(dropped?)
            }
        }
    }

    /// URL of this database, usable as `DATABASE_URL`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Whether this database is SQLite or Postgres.
    pub fn backend(&self) -> DbBackend {
        self.backend
    }

    /// Adds one more empty database of the same kind (e.g. a tenant's dedicated placement in multi-tenant
    /// mode); returns its URL. Extra databases are deleted together with the main one.
    pub async fn extra_database(&self) -> Result<String, TestDatabaseError> {
        let target = match &self.cleanup {
            Cleanup::SqliteDirectory(_) => Target::Sqlite,
            Cleanup::PostgresDatabase { admin_url, .. } => Target::Postgres(admin_url),
        };
        let extra = Self::create(target).await?;
        let url = extra.url.clone();
        self.extras.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(extra.cleanup);
        Ok(url)
    }

    /// Opens a connection pool to this database.
    pub async fn connect(&self) -> Result<DatabaseConnection, TestDatabaseError> {
        let mut options = ConnectOptions::new(self.url.clone());
        // Nhiều test chạy song song, mỗi test một pool: giữ pool nhỏ để không
        // chạm giới hạn kết nối của Postgres.
        options.max_connections(4).sqlx_logging(false);
        Ok(Database::connect(options).await?)
    }
}

/// Deletes test databases that are too old (see module `stale`). Returns the number of databases deleted.
/// SQLite: looks under `sqlite_parent` (the system temp directory; this function's own tests use a separate
/// directory).
async fn remove_stale(
    target: Target<'_>,
    sqlite_parent: &std::path::Path,
    now: SystemTime,
) -> Result<usize, TestDatabaseError> {
    // Thư mục tạm dọn với MỌI loại database: test chạy trên Postgres vẫn tạo thư mục tạm
    // (nơi lưu file, bản backup…) dưới `temp_dir`.
    let directories = sqlite::remove_stale_directories(sqlite_parent, now)
        + template::remove_stale_sqlite(sqlite_parent, now);
    match target {
        Target::Sqlite => Ok(directories),
        Target::Postgres(admin_url) => {
            let admin = postgres::connect_admin(admin_url).await?;
            let removed = postgres::remove_stale_databases(&admin, now).await;
            let templates = template::remove_stale_postgres(&admin, now).await;
            admin.close().await?;
            Ok(directories + removed? + templates?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stale::STALE_AFTER;
    use crate::stale::tests::uuid_created_at;
    use std::time::Duration;

    #[test]
    fn khong_cau_hinh_thi_la_sqlite() {
        for configured in [None, Some(""), Some("  "), Some("sqlite::memory:")] {
            assert_eq!(Target::parse(configured).unwrap(), Target::Sqlite, "{configured:?}");
        }
    }

    #[test]
    fn url_postgres_thi_la_postgres() {
        for url in ["postgres://u:p@h:5432/postgres", "postgresql://h/db"] {
            assert_eq!(Target::parse(Some(url)).unwrap(), Target::Postgres(url));
        }
        // Khoảng trắng thừa quanh URL (hay gặp khi đặt biến môi trường) được bỏ.
        assert_eq!(
            Target::parse(Some(" postgres://h/db\n")).unwrap(),
            Target::Postgres("postgres://h/db")
        );
    }

    #[test]
    fn database_khong_ho_tro_bi_tu_choi_va_khong_lo_mat_khau() {
        let message = Target::parse(Some("mysql://u:secret@h/db")).unwrap_err().to_string();
        assert!(message.contains("must be unset (SQLite) or a postgres:// URL"), "{message}");
        assert!(!message.contains("secret"), "{message}");
    }

    /// Runs on the selected database (SQLite or Postgres): pre-creates an "old"
    /// database then checks it gets cleaned up while the database in use does not.
    #[tokio::test]
    async fn database_cu_bi_don_database_dang_dung_thi_giu() {
        let configured = std::env::var(TEST_DATABASE_URL).ok();
        let target = Target::parse(configured.as_deref()).unwrap();
        let now = SystemTime::now();
        let old = uuid_created_at(now - STALE_AFTER - Duration::from_secs(60));

        // SQLite: thư mục cha riêng cho bài test này. Đặt database "cũ" thẳng vào thư mục tạm
        // chung thì tiến trình test khác (chạy song song) dọn rác đúng lúc và xoá mất nó giữa
        // chừng — bài test đỏ ngẫu nhiên.
        let (parent, _) = sqlite::create_directory(&std::env::temp_dir()).unwrap();
        let stale_url = match target {
            Target::Sqlite => {
                let directory = parent.join(format!("{}{old}", sqlite::DIRECTORY_PREFIX));
                std::fs::create_dir_all(&directory).unwrap();
                let file = directory.join("test.sqlite");
                std::fs::write(&file, b"").unwrap();
                // `mode=rw` (không có `c`): không tự tạo lại file nếu đã mất.
                format!("sqlite://{}?mode=rw", file.display())
            }
            Target::Postgres(admin_url) => {
                let admin = postgres::connect_admin(admin_url).await.unwrap();
                let name = postgres::create_database(&admin, old).await.unwrap();
                admin.close().await.unwrap();
                postgres::with_database(admin_url, &name).unwrap()
            }
        };
        let live = TestDatabase::create(target).await.unwrap();
        // SQLite: một database đang dùng nữa trong thư mục cha riêng, phải được giữ.
        let (fresh, _) = sqlite::create_directory(&parent).unwrap();

        remove_stale(target, &parent, now).await.unwrap();

        assert!(Database::connect(stale_url).await.is_err());
        assert!(fresh.exists());
        let connection = live.connect().await.unwrap();
        connection.ping().await.unwrap();
        connection.close().await.unwrap();
        live.destroy().await.unwrap();
        std::fs::remove_dir_all(&parent).unwrap();
    }

    #[tokio::test]
    async fn url_khong_ho_tro_thi_khong_chay_bai_test() {
        let mut ran = false;
        let result = TestDatabase::run_for(Some("mysql://h/db"), async |_| ran = true).await;
        assert!(result.is_err());
        assert!(!ran);
    }
}
