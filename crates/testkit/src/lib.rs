//! Đồ dùng cho test của Autoxide.
//!
//! Thứ chính là [`TestDatabase`]: mỗi test một database riêng, trống, trên
//! SQLite hoặc Postgres tuỳ biến môi trường `TEST_DATABASE_URL`. Nhờ vậy cùng
//! một bài test chạy được trên cả hai database mà không sửa gì (luật số một
//! trong `CLAUDE.md`), và các test chạy song song không giẫm dữ liệu của nhau.
//!
//! ```no_run
//! use autoxide_testkit::TestDatabase;
//!
//! #[tokio::test]
//! async fn ten_bai_test() {
//!     TestDatabase::run(async |database| {
//!         let connection = database.connect().await.unwrap();
//!         // ... dùng `connection` ...
//!     })
//!     .await
//!     .unwrap();
//! }
//! ```
//!
//! Vì sao là `run(...)` bọc quanh bài test chứ không phải một biến tự dọn khi ra
//! khỏi scope (`Drop`): xoá database là việc `async`, mà `Drop` không `await`
//! được. Đứng chờ trong `Drop` thì chặn thread của runtime tokio, trong khi
//! `DROP DATABASE` của Postgres lại chờ mọi kết nối đang bắt tay dở — do chính
//! thread bị chặn đó phục vụ — nên hai bên chờ nhau tới 60 giây (đã gặp và đo
//! ngày 11/11/2026). Còn giao cho thread nền thì tiến trình test thoát trước khi
//! nó xoá xong, để sót database. `run` thì `await` được việc xoá: chắc chắn
//! xong, không chặn ai.
//!
//! Crate này chỉ được khai ở `[dev-dependencies]` (`cargo xtask check` kiểm).

pub mod ai;
mod app;
mod contract;
mod dns;
mod einvoice;
mod identity;
mod leak;
mod ownership;
mod payment;
mod postgres;
mod push;
mod s3;
mod schema;
mod sqlite;
mod stale;
mod storage;
mod store;
mod tenancy;

pub use app::{TestApp, TestAppBuilder, TestAppError, TestRequest, TestResponse, tool};
pub use dns::FakeDns;
pub use einvoice::{FAKE_EINVOICE_PROVIDER, FakeEInvoiceProvider};
pub use identity::{FAKE_IDENTITY_PROVIDER, FakeIdentityVerifier, FakeSmsSender};
pub use leak::{branch_leak_check, cursor_leak_check, leak_check};
pub use ownership::schema_ownership_violations;
pub use payment::{
    FAKE_PROVIDER, FAKE_RATE_PROVIDER, FakeBankFeed, FakeExchangeRateSource, FakePaymentGateway,
    FakePayoutGateway, FakePayoutOutcome,
};
pub use push::{FakeMobilePushSender, FakeWebPushSender, FakeZnsSender};
pub use s3::{
    TEST_S3_ACCESS_KEY_ID, TEST_S3_BUCKET, TEST_S3_ENDPOINT, TEST_S3_SECRET_ACCESS_KEY, s3_settings,
};
pub use schema::{ColumnSnapshot, IndexSnapshot, SchemaSnapshot, TableSnapshot, schema_snapshot};
pub use storage::{MemoryStorage, MemoryStorageOpener};
pub use store::FakeStorePurchases;
pub use tenancy::{FakeTenantSettings, schema_tenancy_violations};

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbBackend, DbErr};

/// Tên biến môi trường chọn database cho test.
///
/// - Không đặt: SQLite, mỗi test một file trong thư mục tạm.
/// - `postgres://user:password@host:port/<database bất kỳ>`: Postgres. Người dùng
///   trong URL phải có quyền `CREATE DATABASE`; mỗi test tạo một database riêng.
pub const TEST_DATABASE_URL: &str = "TEST_DATABASE_URL";

/// Lỗi khi dựng database cho test.
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

/// Nơi tạo database test, đọc ra từ [`TEST_DATABASE_URL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target<'a> {
    Sqlite,
    /// Kèm URL quản trị (URL người dùng đặt trong biến môi trường).
    Postgres(&'a str),
}

impl<'a> Target<'a> {
    /// `None` hoặc chuỗi rỗng nghĩa là SQLite.
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

/// Một database trống dành riêng cho một test. Lấy qua [`TestDatabase::run`].
#[derive(Debug)]
pub struct TestDatabase {
    url: String,
    backend: DbBackend,
    cleanup: Cleanup,
    /// Database thêm của cùng bài test ([`TestDatabase::extra_database`]), dọn cùng lúc.
    extras: std::sync::Mutex<Vec<Cleanup>>,
}

/// Việc phải dọn sau khi bài test chạy xong.
#[derive(Debug)]
enum Cleanup {
    /// Xoá thư mục tạm chứa file SQLite (kèm file `-wal`, `-shm`).
    SqliteDirectory(PathBuf),
    /// `DROP DATABASE` qua kết nối quản trị.
    PostgresDatabase { admin_url: String, name: String },
}

/// Đã dọn rác của các lần chạy trước trong tiến trình này chưa.
static STALE_REMOVED: AtomicBool = AtomicBool::new(false);

impl TestDatabase {
    /// Tạo một database trống theo biến môi trường [`TEST_DATABASE_URL`], chạy
    /// `test` với nó, xoá database, rồi trả về kết quả của `test`.
    ///
    /// `test` là một async closure: `async |database| { ... }`. Nếu `test` panic
    /// (bài test hỏng) thì database KHÔNG bị xoá ngay; lần chạy test sau sẽ dọn
    /// khi nó đã quá cũ (xem module `stale`).
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

    /// Như [`run`](Self::run) nhưng nhận URL trực tiếp thay vì đọc môi trường.
    /// `None` hoặc chuỗi rỗng nghĩa là SQLite. Dùng khi một bài test cần database
    /// thuộc một loại cụ thể.
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

    /// Xoá database. Nhận `self` (không phải `&self`): sau lời gọi này giá trị
    /// không còn dùng được nữa — trình biên dịch chặn việc dùng database đã xoá.
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

    /// URL của database này, dùng được như `DATABASE_URL`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Database này là SQLite hay Postgres.
    pub fn backend(&self) -> DbBackend {
        self.backend
    }

    /// Thêm một database trống cùng loại (vd placement riêng của một khách ở chế độ nhiều khách);
    /// trả URL của nó. Database thêm bị xoá cùng lúc với database chính.
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

    /// Mở một pool kết nối tới database này.
    pub async fn connect(&self) -> Result<DatabaseConnection, TestDatabaseError> {
        let mut options = ConnectOptions::new(self.url.clone());
        // Nhiều test chạy song song, mỗi test một pool: giữ pool nhỏ để không
        // chạm giới hạn kết nối của Postgres.
        options.max_connections(4).sqlx_logging(false);
        Ok(Database::connect(options).await?)
    }
}

/// Xoá database test đã quá cũ (xem module `stale`). Trả về số database đã xoá. SQLite: tìm
/// dưới `sqlite_parent` (thư mục tạm của hệ thống; test của chính hàm này dùng thư mục riêng).
async fn remove_stale(
    target: Target<'_>,
    sqlite_parent: &std::path::Path,
    now: SystemTime,
) -> Result<usize, TestDatabaseError> {
    // Thư mục tạm dọn với MỌI loại database: test chạy trên Postgres vẫn tạo thư mục tạm
    // (nơi lưu file, bản backup…) dưới `temp_dir`.
    let directories = sqlite::remove_stale_directories(sqlite_parent, now);
    match target {
        Target::Sqlite => Ok(directories),
        Target::Postgres(admin_url) => {
            let admin = postgres::connect_admin(admin_url).await?;
            let removed = postgres::remove_stale_databases(&admin, now).await;
            admin.close().await?;
            Ok(directories + removed?)
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

    /// Chạy trên database đang chọn (SQLite hoặc Postgres): đặt sẵn một database
    /// "cũ" rồi kiểm nó bị dọn, còn database đang dùng thì không.
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
