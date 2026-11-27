//! Reads with a time limit ([`Sql::within`](super::Sql::within)): the database itself stops the query when it
//! runs past the limit, and when the caller stops waiting (the future is dropped), so a slow query never keeps a
//! connection busy after nobody needs its answer.
//!
//! - Postgres: a read-only transaction with `SET LOCAL statement_timeout` (only for that transaction; the
//!   connection goes back to the pool unchanged). Stopped: SQLSTATE `57014`.
//! - SQLite: a progress handler on one connection of the read pool, removed before the connection goes back to the
//!   pool. Stopped: `SQLITE_INTERRUPT`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sea_orm::sqlx;
use sea_orm::{
    AccessMode, ConnectionTrait as _, DatabaseConnection, DbBackend, DbErr, QueryResult,
    RuntimeErr, Statement, TransactionTrait as _,
};

use crate::error::Error;

/// Code of the error a read with a time limit returns when the database stopped it
/// ([`is_query_timeout`]).
pub const QUERY_TIMEOUT: &str = "db.query_timeout";

/// Whether `error` is a read stopped at its time limit ([`QUERY_TIMEOUT`]); the service turns it into its own
/// code.
pub fn is_query_timeout(error: &Error) -> bool {
    error.code() == QUERY_TIMEOUT
}

fn query_timeout() -> Error {
    // Mã viết thẳng (không qua hằng) để `cargo xtask check` thấy nó có bản dịch.
    Error::unavailable("db.query_timeout", "The query ran longer than its time limit.")
}

// SQLite gọi progress handler sau mỗi chừng này lệnh của máy ảo: đủ thưa để không tốn, đủ dày để dừng trong vài ms.
const SQLITE_OPERATIONS: i32 = 1000;
// `SQLITE_INTERRUPT` (mã lỗi chính của SQLite).
const SQLITE_INTERRUPT: &str = "9";
// `query_canceled` của Postgres (cả `statement_timeout`).
const POSTGRES_CANCELED: &str = "57014";

/// Runs `statement` on the read connection, stopped by the database after `limit`.
pub(crate) async fn all_within(
    connection: &DatabaseConnection,
    statement: Statement,
    limit: Duration,
) -> Result<Vec<QueryResult>, Error> {
    match connection.get_database_backend() {
        DbBackend::Postgres => postgres(connection, statement, limit).await,
        DbBackend::Sqlite => sqlite(connection, statement, limit).await,
        backend => Err(Error::internal(format!("no time-limited read on {backend:?}"))),
    }
}

async fn postgres(
    connection: &DatabaseConnection,
    statement: Statement,
    limit: Duration,
) -> Result<Vec<QueryResult>, Error> {
    let millis = limit.as_millis().max(1);
    let transaction = connection.begin_with_config(None, Some(AccessMode::ReadOnly)).await?;
    // `SET LOCAL`: chỉ trong transaction này; kết nối về pool với cấu hình cũ. Số nguyên do kernel tính, không
    // phải dữ liệu người dùng.
    transaction.execute_unprepared(&format!("SET LOCAL statement_timeout = {millis}")).await?;
    // Tiến trình server của kết nối này: người gọi thôi chờ thì huỷ câu của nó (sqlx không tự huỷ khi future bị
    // drop; không huỷ thì câu chạy tới trần).
    let pid = transaction
        .query_one_raw(Statement::from_string(DbBackend::Postgres, "SELECT pg_backend_pid()"))
        .await?
        .ok_or_else(|| Error::internal("pg_backend_pid() returned no row"))?
        .try_get_by_index::<i32>(0)?;
    let mut cancel = CancelOnDrop { connection: Some(connection.clone()), pid };
    let rows = transaction.query_all_raw(statement).await;
    cancel.connection = None;
    transaction.rollback().await?;
    rows.map_err(|error| {
        if code(&error).as_deref() == Some(POSTGRES_CANCELED) {
            query_timeout()
        } else {
            error.into()
        }
    })
}

// Drop khi câu Postgres còn chạy (người gọi thôi chờ): gửi `pg_cancel_backend` cho tiến trình của nó trên một
// kết nối khác của pool.
struct CancelOnDrop {
    connection: Option<DatabaseConnection>,
    pid: i32,
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else { return };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
        // `pid` là số nguyên server trả, không phải dữ liệu người dùng.
        let sql = format!("SELECT pg_cancel_backend({})", self.pid);
        runtime.spawn(async move {
            if let Err(error) = connection.execute_unprepared(&sql).await {
                tracing::warn!(%error, "could not cancel a time-limited read");
            }
        });
    }
}

// Mã lỗi của database trong lỗi của SeaORM.
fn code(error: &DbErr) -> Option<String> {
    match error {
        DbErr::Query(RuntimeErr::SqlxError(error)) | DbErr::Exec(RuntimeErr::SqlxError(error)) => {
            database_code(error)
        }
        _ => None,
    }
}

fn database_code(error: &sqlx::Error) -> Option<String> {
    error.as_database_error().and_then(|error| error.code()).map(|code| code.into_owned())
}

async fn sqlite(
    connection: &DatabaseConnection,
    statement: Statement,
    limit: Duration,
) -> Result<Vec<QueryResult>, Error> {
    let arguments = sqlite_arguments(statement.values.map(|values| values.0).unwrap_or_default())?;
    let pool = connection.get_sqlite_connection_pool();
    let stop = Arc::new(AtomicBool::new(false));
    let mut guard =
        Guard { connection: Some(pool.acquire().await.map_err(sqlx_error)?), stop: stop.clone() };
    let Some(pooled) = guard.connection.as_mut() else {
        return Err(Error::internal("time-limited read lost its connection"));
    };
    let deadline = Instant::now() + limit;
    {
        let mut handle = pooled.lock_handle().await.map_err(sqlx_error)?;
        // Trả `false` thì SQLite dừng câu đang chạy (`SQLITE_INTERRUPT`). Kiểm cả cờ `stop`: future bị drop.
        handle.set_progress_handler(SQLITE_OPERATIONS, move || {
            !stop.load(Ordering::Relaxed) && Instant::now() < deadline
        });
    }
    let rows = sqlx::query_with(sqlx::AssertSqlSafe(statement.sql.as_str()), arguments)
        .fetch_all(&mut **pooled)
        .await;
    // Gỡ handler trước khi kết nối về pool: câu sau trên kết nối này không bị dừng nhầm.
    pooled.lock_handle().await.map_err(sqlx_error)?.remove_progress_handler();
    guard.finished();
    match rows {
        Ok(rows) => Ok(rows.into_iter().map(QueryResult::from).collect()),
        Err(error) if database_code(&error).as_deref() == Some(SQLITE_INTERRUPT) => {
            Err(query_timeout())
        }
        Err(error) => Err(sqlx_error(error)),
    }
}

fn sqlx_error(error: sqlx::Error) -> Error {
    Error::from(DbErr::Query(RuntimeErr::SqlxError(Arc::new(error))))
}

// Kết nối đang chạy câu có trần thời gian. Drop trước `finished` (người gọi thôi chờ): bật cờ
// `stop` để SQLite dừng câu ngay, và đóng kết nối thay vì trả về pool (handler còn gắn trên nó).
struct Guard {
    connection: Option<sqlx::pool::PoolConnection<sqlx::Sqlite>>,
    stop: Arc<AtomicBool>,
}

impl Guard {
    fn finished(&mut self) {
        // Trả kết nối về pool bình thường (handler đã gỡ).
        self.connection.take();
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut connection) = self.connection.take() {
            self.stop.store(true, Ordering::Relaxed);
            connection.close_on_drop();
        }
    }
}

// Giá trị của câu cho sqlx (SQLite). Chỉ các kiểu câu đọc dùng; kiểu khác là lỗi nội bộ, không đoán.
fn sqlite_arguments(values: Vec<sea_orm::Value>) -> Result<sqlx::sqlite::SqliteArguments, Error> {
    use sea_orm::Value;
    use sqlx::Arguments as _;
    let mut arguments = sqlx::sqlite::SqliteArguments::default();
    for value in values {
        let added = match value {
            Value::Bool(value) => arguments.add(value),
            Value::SmallInt(value) => arguments.add(value),
            Value::Int(value) => arguments.add(value),
            Value::BigInt(value) => arguments.add(value),
            Value::String(value) => arguments.add(value),
            Value::Bytes(value) => arguments.add(value),
            Value::Uuid(value) => arguments.add(value),
            other => {
                return Err(Error::internal(format!(
                    "time-limited read cannot bind {other:?} on SQLite"
                )));
            }
        };
        added.map_err(|error| Error::internal(format!("bind: {error}")))?;
    }
    Ok(arguments)
}
