//! Database connections: separate READ and WRITE connections.
//!
//! Why they are separate (docs/DECISIONS.md D-009):
//!
//! - **SQLite is single-writer**: only one write statement at a time. The write side therefore uses exactly
//!   ONE connection: writes queue up inside the application instead of fighting over the lock and getting
//!   "database is locked" errors. The read side uses many connections; thanks to WAL, reads are not blocked
//!   by writes.
//! - **Postgres** uses the same calls: the write side is a pool to the main database, the read side is
//!   another pool that can point at a read replica.
//!
//! Read connections are forbidden to write on BOTH databases (SQLite opens read-only; Postgres sets
//! `default_transaction_read_only`). A write through [`Database::read`] fails immediately in tests, on either
//! database, instead of silently working on one side.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use sea_orm::sqlx::sqlite::{SqliteJournalMode, SqliteSynchronous};
use sea_orm::{
    AccessMode, ConnectOptions, DatabaseConnection, DatabaseTransaction, DbErr, IsolationLevel,
    RuntimeErr, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};

use crate::config::{DatabaseBackend, DatabaseConfig, DatabaseUrl};
use crate::error::Error;

/// Maximum number of runs of [`Database::write_with_retry`] (the first run plus 4 reruns).
pub const RETRY_ATTEMPTS: u32 = 5;

/// Error when opening a database connection.
#[derive(Debug)]
pub struct DatabaseError(String);

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database: {}", self.0)
    }
}

impl std::error::Error for DatabaseError {}

/// The two connection pools to the database: one for reading, one for writing.
///
/// `Clone` is cheap: inside is a reference-counted pointer to the pools, no new connection is opened.
#[derive(Debug, Clone)]
pub struct Database {
    read: DatabaseConnection,
    write: DatabaseConnection,
    backend: DatabaseBackend,
}

impl Database {
    /// Opens the two pools according to `config`. For SQLite: creates the directory and file if missing,
    /// enables WAL and foreign keys.
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

    /// A database that connects nowhere: every query fails. Only for kernel unit tests that need a
    /// [`TenantContext`](crate::tenancy::TenantContext) without running any query.
    #[cfg(test)]
    pub(crate) fn detached() -> Self {
        Database {
            read: DatabaseConnection::default(),
            write: DatabaseConnection::default(),
            backend: DatabaseBackend::Sqlite,
        }
    }

    /// Calls `observer` with the SQL of every statement run through the two pools (including inside a
    /// transaction), after the statement has run. Used to count SQL statements per request in tests and
    /// benchmarks (D-20261120-step39-hot-paths); do not enable it on a real server.
    ///
    /// Must be called BEFORE cloning `Database` for anyone else: each clone keeps the observer it had when
    /// cloned.
    pub fn observe_statements(&mut self, observer: Arc<dyn Fn(&str) + Send + Sync>) {
        // `move` + clone `Arc`: mỗi pool giữ một con trỏ tới cùng một observer.
        let read = Arc::clone(&observer);
        self.read.set_metric_callback(move |info| read(&info.statement.sql));
        self.write.set_metric_callback(move |info| observer(&info.statement.sql));
    }

    /// Connection for READ statements. A write through this connection is rejected by the database.
    pub fn read(&self) -> &DatabaseConnection {
        &self.read
    }

    /// Connection for WRITE statements. A single write statement uses this connection directly; several
    /// statements that must succeed or fail together use [`begin_write`](Self::begin_write).
    pub fn write(&self) -> &DatabaseConnection {
        &self.write
    }

    /// Opens a write transaction. Call `.commit().await` to record it; a transaction that is
    /// dropped without a commit rolls back automatically.
    ///
    /// Keep the transaction SHORT: no network calls, no password hashing inside it, because on
    /// SQLite every other write is waiting for it.
    ///
    /// On SQLite the transaction opens with `BEGIN IMMEDIATE`: it takes the write permission right
    /// away. The default mode (`DEFERRED`) asks for write permission only at the first write
    /// statement, and if another process has just written by then, the transaction fails midway
    /// instead of waiting.
    pub async fn begin_write(&self) -> Result<DatabaseTransaction, DbErr> {
        let options = TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            // `..Default::default()`: các trường còn lại lấy giá trị mặc định.
            ..Default::default()
        };
        self.write.begin_with_options(options).await
    }

    /// Reruns the write operation `work` on a TRANSIENT conflict, at most [`RETRY_ATTEMPTS`] times
    /// (D-182). `work` opens a transaction itself with [`begin_write`](Self::begin_write), does its work and
    /// commits; an error at any step makes the transaction roll back (dropped without a commit).
    ///
    /// A transient conflict is an error that a rerun may fix: Postgres `40001` (serialization
    /// failure) and `40P01` (deadlock: Postgres cancels one of two transactions waiting on each other); SQLite
    /// busy (`SQLITE_BUSY`, `SQLITE_LOCKED`: another process holds the write permission for longer than
    /// `AUTOXIDE_DATABASE_WAIT_SECONDS`). Every other error, including a business error `work` returns itself,
    /// is returned immediately without a rerun.
    ///
    /// **`work` must tolerate being rerun**: every database change of the previous run was rolled back,
    /// but anything outside the database was not, so do not send mail, call the network or change outside
    /// variables inside `work`; reread everything in the new transaction instead of using values read earlier.
    /// The waits between runs are short, growing, and randomly jittered so the two sides stop colliding.
    ///
    /// ```ignore
    /// // `move` + references: each call of the closure creates a new future borrowing the same data.
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

    /// Opens a READ-ONLY transaction that sees the database as of one point in time: every read inside
    /// gives the same snapshot, even while other writes run (used for backup and data copy). Postgres:
    /// `REPEATABLE READ READ ONLY`; SQLite (WAL): a plain read transaction
    /// already fixes the snapshot from its first read.
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

    /// Whether the database in use is SQLite or Postgres.
    pub fn backend(&self) -> DatabaseBackend {
        self.backend
    }

    /// Checks that both sides still answer. Used for health checks.
    pub async fn ping(&self) -> Result<(), DbErr> {
        self.write.ping().await?;
        self.read.ping().await
    }

    /// Closes both pools and waits for the connections to close fully. Call on graceful shutdown.
    pub async fn close(self) -> Result<(), DbErr> {
        // Đóng phía đọc trước: trên SQLite, kết nối ĐÓNG CUỐI CÙNG là kết nối gộp
        // file WAL vào file chính, và việc đó cần quyền ghi.
        self.read.close().await?;
        self.write.close().await
    }
}

/// Whether `error` is a transient conflict where rerunning the transaction may succeed (see
/// [`Database::write_with_retry`]). Walks the cause chain looking for the database's error.
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

/// The database error code (Postgres SQLSTATE, SQLite result code) in `error`, if any.
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

/// Wait before run number `attempt + 1`: 10 ms, 20 ms, 40 ms, 80 ms... plus a random jitter up to the same
/// amount, so two transactions that just collided do not rerun at the same moment.
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
    match sea_orm::Database::connect(options).await {
        Ok(connection) => Ok(connection),
        Err(error) => {
            // Thông báo dùng URL đã che mật khẩu; lỗi của driver không chứa mật khẩu.
            let mut message = format!("cannot connect to `{}`: {error}", url.redacted());
            // Sai cổng hay máy chưa chạy thì driver chỉ báo "pool timed out" sau chục giây, giấu nguyên nhân thật
            // ("connection refused"): thử mở TCP một lần để nói nguyên nhân gốc.
            if let Some(cause) = tcp_cause(url).await {
                message.push_str(&format!(" (root cause: {cause})"));
            }
            Err(DatabaseError(message))
        }
    }
}

// Nếu không mở được kết nối TCP tới máy chủ trong URL PostgreSQL thì trả lý do (đã nói máy và cổng); ngược lại `None`.
async fn tcp_cause(url: &DatabaseUrl) -> Option<String> {
    if url.backend() != DatabaseBackend::Postgres {
        return None;
    }
    let parsed = url::Url::parse(url.expose()).ok()?;
    let host = parsed.host_str()?.to_owned();
    let port = parsed.port().unwrap_or(5432);
    let attempt = tokio::time::timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await;
    match attempt {
        Ok(Ok(_)) => None,
        Ok(Err(error)) => Some(format!("cannot reach {host}:{port}: {error}")),
        Err(_) => Some(format!("no answer from {host}:{port} within 3 seconds")),
    }
}

/// A dedicated connection to the main database, outside the two pools: the migration lock on Postgres
/// ([`MigrationLock`](crate::migration::MigrationLock)) keeps its transaction open for the whole migration
/// without taking a connection from the write pool.
pub(crate) async fn dedicated_connection(
    config: &DatabaseConfig,
) -> Result<DatabaseConnection, DatabaseError> {
    let mut options = base_options(config, &config.url);
    options.max_connections(1).min_connections(1);
    open(options, &config.url).await
}

/// The part shared by both sides.
fn base_options(config: &DatabaseConfig, url: &DatabaseUrl) -> ConnectOptions {
    let mut options = ConnectOptions::new(url.expose().to_owned());
    options
        .acquire_timeout(config.wait)
        // Log câu lệnh SQL do mục "Log, metrics, tracing" của lộ trình quyết định;
        // mặc định của thư viện là in mọi câu lệnh ở mức INFO.
        .sqlx_logging(false);
    options
}

/// Number of rows each SQLite index reads when gathering statistics (`PRAGMA analysis_limit`): the level SQLite
/// recommends, enough for the planner, a few hundred milliseconds even for tables with millions of rows.
const ANALYSIS_LIMIT: u32 = 400;

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
                        // Thống kê cho bộ lập kế hoạch truy vấn (bước 39, D-20261118-step39-sqlite-optimize):
                        // không có `sqlite_stat1` thì SQLite có thể chọn sai index — trên 1 triệu
                        // bình luận, luồng bình luận quét theo `(tenant_id, id)` mất 0,4 s thay vì
                        // 0,4 ms. Postgres tự `ANALYZE` (autovacuum); SQLite thì không. `optimize`
                        // với cờ 0x10002 lúc mở kết nối ghi: xét MỌI bảng, chỉ phân tích bảng cần,
                        // mỗi index đọc tối đa `analysis_limit` dòng nên nhanh cả khi bảng lớn.
                        // Đóng kết nối thì chạy lại (theo thay đổi từ lúc mở).
                        .analysis_limit(ANALYSIS_LIMIT)
                        .pragma("optimize", "0x10002")
                        .optimize_on_close(true, ANALYSIS_LIMIT)
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

    // Cổng 1 không có ai nghe: nguyên nhân gốc là "không tới được", không phải "pool timed out" chung chung.
    #[tokio::test]
    async fn postgres_sai_cong_noi_nguyen_nhan_goc() {
        let url: DatabaseUrl = "postgres://u:p@127.0.0.1:1/db".parse().unwrap();
        let cause = tcp_cause(&url).await.unwrap();
        assert!(cause.contains("127.0.0.1:1"), "{cause}");
        let sqlite: DatabaseUrl = "sqlite://data/x.sqlite".parse().unwrap();
        assert_eq!(tcp_cause(&sqlite).await, None);
    }

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

/// A hand-written SQL statement with parameters: use it INSTEAD OF `sea_orm::Statement::from_sql_and_values`
/// (`cargo xtask check` rejects the old way, rule `raw-statement`).
///
/// Why: the kernel's time values (`Timestamp`, `Date`) reach the driver as a string with a type name
/// (`timestamptz`, `date`); SQLite needs exactly the string we wrote. The SeaORM / sea-query builder
/// inserts the type name into Postgres statements itself (`$1::"timestamptz"`); for a hand-written
/// statement nobody does, and Postgres receives a string and rejects it (error 42804). This function
/// inserts it exactly as the builder does: each parameter that carries a type name turns `$n` into
/// `$n::"<type>"` (Postgres only; SQLite is left unchanged). Do not write `CAST(? AS timestamptz)`
/// yourself: SQLite reads an unknown type as a number.
///
/// Write Postgres statements with `$1`, `$2`... as usual; a `$n` string inside a `'…'` literal is rewritten
/// too, so do not write `$` in a literal of a statement that has time parameters.
pub fn statement<I>(
    backend: sea_orm::DbBackend,
    sql: impl Into<String>,
    values: I,
) -> sea_orm::Statement
where
    I: IntoIterator<Item = sea_orm::Value>,
{
    let values: Vec<sea_orm::Value> = values.into_iter().collect();
    let mut sql: String = sql.into();
    if backend == sea_orm::DbBackend::Postgres {
        for (index, value) in values.iter().enumerate() {
            let type_name = match value {
                sea_orm::Value::Enum(sea_orm::sea_query::OptionEnum::Some(inner)) => {
                    inner.type_name.to_string()
                }
                sea_orm::Value::Enum(sea_orm::sea_query::OptionEnum::None(type_name)) => {
                    type_name.to_string()
                }
                _ => continue,
            };
            sql = cast_placeholder(&sql, index + 1, &type_name);
        }
    }
    sea_orm::Statement::from_sql_and_values(backend, sql, values)
}

/// Appends `::"<type_name>"` after each standalone `$<number>` (not the `$1` in `$10`, with no `::` already
/// following).
fn cast_placeholder(sql: &str, number: usize, type_name: &str) -> String {
    let placeholder = format!("${number}");
    let mut out = String::with_capacity(sql.len() + 16);
    let mut rest = sql;
    while let Some(position) = rest.find(&placeholder) {
        let after = &rest[position + placeholder.len()..];
        out.push_str(&rest[..position + placeholder.len()]);
        let standalone = !after.starts_with(|c: char| c.is_ascii_digit());
        if standalone && !after.starts_with("::") {
            out.push_str("::\"");
            out.push_str(type_name);
            out.push('"');
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod statement_tests {
    use super::{cast_placeholder, statement};
    use crate::time::{Date, Timestamp};
    use sea_orm::DbBackend;

    #[test]
    fn chen_ten_kieu_dung_cho_tham_so_postgres() {
        assert_eq!(
            cast_placeholder("a = $1 AND b = $10", 1, "date"),
            "a = $1::\"date\" AND b = $10"
        );
        assert_eq!(cast_placeholder("a = $1::text", 1, "date"), "a = $1::text", "đã có kiểu: giữ");
        let at = Timestamp::UNIX_EPOCH;
        let day = Date::new(2026, 11, 21).unwrap();
        let sql = "UPDATE t SET at = $1, day = $2, n = $3 WHERE id = $4";
        let values = || [at.into(), day.into(), 1_i64.into(), None::<Timestamp>.into()];
        assert_eq!(
            statement(DbBackend::Postgres, sql, values()).sql,
            "UPDATE t SET at = $1::\"timestamptz\", day = $2::\"date\", n = $3 WHERE id = $4::\"timestamptz\""
        );
        assert_eq!(statement(DbBackend::Sqlite, sql, values()).sql, sql, "SQLite giữ nguyên");
    }
}
