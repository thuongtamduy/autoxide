//! Kết nối database: tách kết nối ĐỌC và kết nối GHI.
//!
//! Vì sao tách (docs/DECISIONS.md D-009):
//!
//! - **SQLite là single-writer** — chỉ một lệnh ghi tại một thời điểm. Phía ghi
//!   vì thế dùng đúng MỘT kết nối: các lệnh ghi xếp hàng trong ứng dụng thay vì
//!   tranh lock và nhận lỗi "database is locked". Phía đọc dùng nhiều kết nối;
//!   nhờ WAL, đọc không bị lệnh ghi chặn.
//! - **Postgres** dùng cùng cách gọi: phía ghi là một pool tới database chính,
//!   phía đọc là một pool khác, trỏ được sang read replica.
//!
//! Kết nối đọc bị cấm ghi trên CẢ HAI database (SQLite mở read-only; Postgres đặt
//! `default_transaction_read_only`). Lỡ ghi qua [`Database::read`] thì hỏng ngay
//! ở test, trên database nào cũng vậy — chứ không lặng lẽ chạy được ở một bên.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{
    AccessMode, ConnectOptions, DatabaseConnection, DatabaseTransaction, DbErr, IsolationLevel,
    RuntimeErr, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

use crate::config::{DatabaseBackend, DatabaseConfig, DatabaseUrl};
use crate::error::Error;

/// Số lần chạy tối đa của [`Database::write_with_retry`] (lần đầu + 4 lần chạy lại).
pub const RETRY_ATTEMPTS: u32 = 5;

/// Lỗi khi mở kết nối database.
#[derive(Debug)]
pub struct DatabaseError(String);

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database: {}", self.0)
    }
}

impl std::error::Error for DatabaseError {}

/// Hai pool kết nối tới database: một để đọc, một để ghi.
///
/// `Clone` rẻ: bên trong là con trỏ đếm tham chiếu tới pool, không mở thêm kết nối.
#[derive(Debug, Clone)]
pub struct Database {
    read: DatabaseConnection,
    write: DatabaseConnection,
    backend: DatabaseBackend,
}

impl Database {
    /// Mở hai pool theo `config`. Với SQLite: tạo thư mục và file nếu chưa có, bật
    /// WAL và foreign key.
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let backend = config.url.backend();

        if let Some(directory) = config.url.sqlite_path().as_deref().and_then(|path| path.parent())
        {
            // Gọi hệ thống file kiểu đồng bộ trong hàm async: chấp nhận được vì chỉ
            // chạy một lần lúc khởi động, trước khi phục vụ request nào.
            std::fs::create_dir_all(directory).map_err(|error| {
                DatabaseError(format!(
                    "cannot create directory `{}` for the SQLite file: {error}",
                    directory.display()
                ))
            })?;
        }

        // Mở phía ghi TRƯỚC: trên SQLite chính nó tạo file và chuyển file sang WAL;
        // phía đọc (read-only) không làm được hai việc đó.
        let write = open(write_options(config), &config.url).await?;
        let read_url = config.read_url.as_ref().unwrap_or(&config.url);
        let read = open(read_options(config, read_url), read_url).await?;

        Ok(Database { read, write, backend })
    }

    /// Database không nối tới đâu: mọi truy vấn lỗi. Chỉ cho test đơn vị của kernel cần một
    /// [`TenantContext`](crate::tenancy::TenantContext) mà không truy vấn gì.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Database {
            read: DatabaseConnection::default(),
            write: DatabaseConnection::default(),
            backend: DatabaseBackend::Sqlite,
        }
    }

    /// Kết nối cho lệnh ĐỌC. Ghi qua kết nối này bị database từ chối.
    pub fn read(&self) -> &DatabaseConnection {
        &self.read
    }

    /// Kết nối cho lệnh GHI. Một lệnh ghi đơn lẻ dùng thẳng kết nối này; nhiều
    /// lệnh phải cùng thành công hoặc cùng thất bại thì dùng
    /// [`begin_write`](Self::begin_write).
    pub fn write(&self) -> &DatabaseConnection {
        &self.write
    }

    /// Mở một transaction ghi. Gọi `.commit().await` để ghi nhận; transaction bị
    /// drop mà chưa commit thì tự rollback.
    ///
    /// Giữ transaction NGẮN: không gọi mạng, không hash mật khẩu bên trong — trên
    /// SQLite mọi lệnh ghi khác đang đứng chờ nó.
    ///
    /// Trên SQLite transaction mở bằng `BEGIN IMMEDIATE`: lấy quyền ghi ngay từ
    /// đầu. Kiểu mặc định (`DEFERRED`) chỉ xin quyền ghi khi gặp lệnh ghi đầu
    /// tiên, và nếu lúc đó một tiến trình khác vừa ghi thì transaction hỏng giữa
    /// chừng thay vì chờ.
    pub async fn begin_write(&self) -> Result<DatabaseTransaction, DbErr> {
        let options = TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            // `..Default::default()`: các trường còn lại lấy giá trị mặc định.
            ..Default::default()
        };
        self.write.begin_with_options(options).await
    }

    /// Chạy lại thao tác ghi `work` khi gặp xung đột TẠM THỜI, tối đa [`RETRY_ATTEMPTS`] lần
    /// (D-182). `work` tự mở transaction bằng [`begin_write`](Self::begin_write), làm việc và
    /// commit; lỗi ở bất kỳ bước nào làm transaction tự rollback (bị drop chưa commit).
    ///
    /// Xung đột tạm thời là lỗi mà chạy lại có thể thành công: Postgres `40001` (serialization
    /// failure) và `40P01` (deadlock — Postgres huỷ một trong hai transaction chờ nhau); SQLite
    /// bận (`SQLITE_BUSY`, `SQLITE_LOCKED`: tiến trình khác giữ quyền ghi quá
    /// `AUTOXIDE_DATABASE_WAIT_SECONDS`). Mọi lỗi khác — kể cả lỗi nghiệp vụ `work` tự trả — trả
    /// về ngay, không chạy lại.
    ///
    /// **`work` phải chịu được chạy lại**: mọi thay đổi database của lần trước đã bị rollback,
    /// nhưng việc ngoài database thì không — đừng gửi mail, gọi mạng hay đổi biến bên ngoài
    /// trong `work`; đọc lại mọi thứ trong transaction mới thay vì dùng giá trị đọc trước đó.
    /// Giữa các lần chờ ngắn, tăng dần, có độ lệch ngẫu nhiên để hai bên không đụng nhau nữa.
    ///
    /// ```ignore
    /// // `move` + tham chiếu: mỗi lần gọi closure tạo một future mới mượn cùng dữ liệu.
    /// let (database, order) = (&self.database, &order);
    /// let paid = database
    ///     .write_with_retry(move || async move {
    ///         let transaction = database.begin_write().await?;
    ///         let paid = pay(&transaction, order).await?;
    ///         transaction.commit().await?;
    ///         Ok(paid)
    ///     })
    ///     .await?;
    /// ```
    pub async fn write_with_retry<T, F, Work>(&self, mut work: F) -> Result<T, Error>
    where
        // `FnMut() -> Work`: closure gọi được nhiều lần, mỗi lần trả một future mới (một lần
        // chạy). Không nhận transaction làm tham số: future mượn transaction thì kiểu của nó phải
        // mang thời gian sống của transaction, và Rust hiện chưa chứng minh được future như vậy
        // là `Send` (handler HTTP và event handler cần `Send`).
        F: FnMut() -> Work,
        Work: Future<Output = Result<T, Error>>,
    {
        let mut attempt = 1;
        loop {
            match work().await {
                Err(error) if attempt < RETRY_ATTEMPTS && is_transient(&error) => {
                    tracing::debug!(attempt, error = %error, "transient write conflict; retrying");
                    metrics::counter!("database_write_retries_total").increment(1);
                    tokio::time::sleep(retry_delay(attempt)).await;
                    attempt += 1;
                }
                outcome => return outcome,
            }
        }
    }

    /// Mở một transaction CHỈ ĐỌC thấy database như ở một thời điểm: mọi câu đọc bên
    /// trong cho cùng một bức ảnh, dù lệnh ghi khác đang chạy (dùng cho backup, chép dữ
    /// liệu). Postgres: `REPEATABLE READ READ ONLY`; SQLite (WAL): transaction đọc thường
    /// đã cố định bức ảnh từ câu đọc đầu tiên.
    pub async fn begin_snapshot(&self) -> Result<DatabaseTransaction, DbErr> {
        match self.backend {
            DatabaseBackend::Postgres => {
                self.read
                    .begin_with_config(
                        Some(IsolationLevel::RepeatableRead),
                        Some(AccessMode::ReadOnly),
                    )
                    .await
            }
            _ => self.read.begin().await,
        }
    }

    /// Database đang dùng là SQLite hay Postgres.
    pub fn backend(&self) -> DatabaseBackend {
        self.backend
    }

    /// Kiểm cả hai phía còn trả lời. Dùng cho health check.
    pub async fn ping(&self) -> Result<(), DbErr> {
        self.write.ping().await?;
        self.read.ping().await
    }

    /// Đóng cả hai pool, chờ các kết nối đóng hẳn. Gọi khi graceful shutdown.
    pub async fn close(self) -> Result<(), DbErr> {
        // Đóng phía đọc trước: trên SQLite, kết nối ĐÓNG CUỐI CÙNG là kết nối gộp
        // file WAL vào file chính, và việc đó cần quyền ghi.
        self.read.close().await?;
        self.write.close().await
    }
}

/// Lỗi `error` có phải xung đột tạm thời, chạy lại transaction có thể thành công không (xem
/// [`Database::write_with_retry`]). Đi dọc chuỗi nguyên nhân tìm lỗi của database.
pub fn is_transient(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if let Some(code) = error.downcast_ref::<DbErr>().and_then(database_code) {
            return is_transient_code(&code);
        }
        current = error.source();
    }
    false
}

/// Mã lỗi database (SQLSTATE của Postgres, mã kết quả của SQLite) trong `error`, nếu có.
fn database_code(error: &DbErr) -> Option<String> {
    let runtime = match error {
        DbErr::Conn(runtime) | DbErr::Exec(runtime) | DbErr::Query(runtime) => runtime,
        _ => return None,
    };
    match runtime {
        RuntimeErr::SqlxError(sqlx) => sqlx
            .as_database_error()
            .and_then(|database| database.code())
            .map(|code| code.into_owned()),
        _ => None,
    }
}

fn is_transient_code(code: &str) -> bool {
    match code {
        // Postgres: serialization_failure, deadlock_detected.
        "40001" | "40P01" => true,
        // SQLite: mã kết quả mở rộng; byte thấp là mã chính — 5 SQLITE_BUSY (cả BUSY_RECOVERY,
        // BUSY_SNAPSHOT, BUSY_TIMEOUT), 6 SQLITE_LOCKED.
        _ => code.parse::<u32>().is_ok_and(|code| matches!(code & 0xff, 5 | 6)),
    }
}

/// Thời gian chờ trước lần chạy thứ `attempt + 1`: 10 ms, 20 ms, 40 ms, 80 ms… cộng một độ lệch
/// ngẫu nhiên tới bằng chính nó, để hai transaction vừa đụng nhau không chạy lại cùng lúc.
fn retry_delay(attempt: u32) -> Duration {
    let base = 10u64 << attempt.saturating_sub(1).min(6);
    let mut byte = [0u8; 1];
    let jitter = match getrandom::fill(&mut byte) {
        Ok(()) => base * u64::from(byte[0]) / 255,
        Err(_) => 0,
    };
    Duration::from_millis(base + jitter)
}

async fn open(
    options: ConnectOptions,
    url: &DatabaseUrl,
) -> Result<DatabaseConnection, DatabaseError> {
    sea_orm::Database::connect(options).await.map_err(|error| {
        // Thông báo dùng URL đã che mật khẩu; lỗi của driver không chứa mật khẩu.
        DatabaseError(format!("cannot connect to `{}`: {error}", url.redacted()))
    })
}

/// Phần chung của hai phía.
fn base_options(config: &DatabaseConfig, url: &DatabaseUrl) -> ConnectOptions {
    let mut options = ConnectOptions::new(url.expose().to_owned());
    options
        .acquire_timeout(config.wait)
        // Log câu lệnh SQL do mục "Log, metrics, tracing" của lộ trình quyết định;
        // mặc định của thư viện là in mọi câu lệnh ở mức INFO.
        .sqlx_logging(false);
    options
}

fn write_options(config: &DatabaseConfig) -> ConnectOptions {
    let mut options = base_options(config, &config.url);
    let wait = config.wait;
    match config.url.backend() {
        DatabaseBackend::Sqlite => {
            options
                // Đúng một kết nối ghi, giữ mở suốt đời tiến trình.
                .max_connections(1)
                .min_connections(1)
                .map_sqlx_sqlite_opts(move |sqlite| {
                    sqlite
                        .create_if_missing(true)
                        // WAL: đọc và ghi không chặn nhau. Chế độ này được lưu
                        // trong file nên kết nối đọc tự thấy.
                        .journal_mode(SqliteJournalMode::Wal)
                        // NORMAL là mức khuyến nghị cho WAL: an toàn trước việc
                        // ứng dụng sập; mất điện có thể mất vài transaction cuối
                        // nhưng không hỏng file.
                        .synchronous(SqliteSynchronous::Normal)
                        // SQLite KHÔNG kiểm foreign key nếu không bật, mỗi kết nối.
                        .foreign_keys(true)
                        // Chờ lock của tiến trình khác (vd api và worker chạy riêng)
                        // thay vì báo lỗi ngay.
                        .busy_timeout(wait)
                });
        }
        DatabaseBackend::Postgres => {
            options
                .max_connections(config.write_connections)
                .map_sqlx_postgres_opts(|postgres| postgres.application_name("autoxide"));
        }
    }
    options
}

fn read_options(config: &DatabaseConfig, url: &DatabaseUrl) -> ConnectOptions {
    let mut options = base_options(config, url);
    options.max_connections(config.read_connections);
    let wait = config.wait;
    match url.backend() {
        DatabaseBackend::Sqlite => {
            options.map_sqlx_sqlite_opts(move |sqlite| {
                sqlite.read_only(true).foreign_keys(true).busy_timeout(wait)
            });
        }
        DatabaseBackend::Postgres => {
            options.map_sqlx_postgres_opts(|postgres| {
                postgres
                    .application_name("autoxide-read")
                    // Mọi transaction trên kết nối này mặc định chỉ đọc.
                    .options([("default_transaction_read_only", "on")])
            });
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_loi_tam_thoi() {
        for code in ["40001", "40P01", "5", "6", "261", "517", "773", "262"] {
            assert!(is_transient_code(code), "{code}");
        }
        for code in ["23505", "23503", "42P01", "19", "2067", "1555", "", "abc"] {
            assert!(!is_transient_code(code), "{code}");
        }
        let plain = Error::conflict("notes.title_taken", "taken");
        assert!(!is_transient(&plain));
    }

    #[test]
    fn thoi_gian_cho_tang_dan() {
        for attempt in 1..RETRY_ATTEMPTS {
            let base = 10u64 << (attempt - 1);
            let delay = retry_delay(attempt).as_millis() as u64;
            assert!((base..=2 * base).contains(&delay), "{attempt}: {delay}");
        }
    }
}
