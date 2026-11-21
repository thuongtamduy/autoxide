//! Khoá migration: chỉ MỘT tiến trình chạy migration của database tại một lúc.
//!
//! Cần khi máy chủ tự migrate lúc khởi động (`AUTOXIDE_MIGRATE_ON_START`,
//! D-20261121-migrate-on-start): nhiều bản máy chủ khởi động cùng lúc thì bản đầu tiên migrate,
//! các bản sau chờ nó xong rồi thấy không còn gì phải làm. `autoxide migrate up` cũng lấy khoá này.
//!
//! Hai database không có chung một cơ chế, nên rẽ nhánh ở đây (sau một kiểu duy nhất,
//! [`MigrationLock`]) và test chạy trên cả hai:
//!
//! - **Postgres**: advisory lock theo transaction (`pg_try_advisory_xact_lock`) trên một kết nối
//!   riêng, transaction giữ mở tới khi nhả khoá. Tiến trình chết thì kết nối đứt, Postgres tự
//!   nhả — không có khoá "mồ côi".
//! - **SQLite**: database là một file trên máy, nên khoá là khoá file của hệ điều hành
//!   (`File::try_lock`) trên file `<database>.migrate-lock` cạnh nó. Tiến trình chết thì hệ điều
//!   hành tự nhả. SQLite trong bộ nhớ chỉ một tiến trình thấy được: không cần khoá.

use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, Statement,
    TransactionTrait,
};

use super::MigrationError;
use crate::config::{DatabaseBackend, DatabaseConfig};

/// Số định danh advisory lock của migration trên Postgres: chữ "autoxide" (8 byte ASCII) đọc thành
/// số nguyên 64 bit. Cố định — mọi phiên bản Autoxide dùng cùng số này.
const POSTGRES_KEY: i64 = i64::from_be_bytes(*b"autoxide");

/// Đuôi thêm vào đường dẫn file SQLite để thành file khoá.
const LOCK_FILE_SUFFIX: &str = ".migrate-lock";

/// Khoảng giữa hai lần thử lấy khoá khi tiến trình khác đang giữ.
const RETRY_EVERY: Duration = Duration::from_millis(250);

/// Khoảng giữa hai dòng log "đang chờ".
const LOG_EVERY: Duration = Duration::from_secs(10);

/// Khoá migration đang giữ. Nhả bằng [`release`](Self::release); bị drop thì cũng nhả (kết nối
/// hay file đóng lại), chỉ là không chờ được Postgres xác nhận.
#[derive(Debug)]
pub struct MigrationLock {
    held: Held,
}

#[derive(Debug)]
enum Held {
    // Giữ cả kết nối lẫn transaction: transaction mượn kết nối từ pool riêng của nó, pool phải
    // sống tới khi transaction kết thúc.
    Postgres {
        transaction: DatabaseTransaction,
        connection: DatabaseConnection,
    },
    // Khoá file nằm trên file handle: đóng file (drop) là nhả.
    File(File),
    /// SQLite trong bộ nhớ.
    Nothing,
}

impl MigrationLock {
    /// Lấy khoá migration của database `config`. Tiến trình khác đang giữ thì chờ, thử lại mỗi
    /// 250 ms, tối đa `wait`; quá hạn thì lỗi nói rõ phải làm gì.
    pub async fn acquire(config: &DatabaseConfig, wait: Duration) -> Result<Self, MigrationError> {
        let held = match config.url.backend() {
            DatabaseBackend::Postgres => acquire_postgres(config, wait).await?,
            DatabaseBackend::Sqlite => match config.url.sqlite_path() {
                Some(path) => acquire_file(lock_path(path), wait).await?,
                None => Held::Nothing,
            },
        };
        Ok(MigrationLock { held })
    }

    /// Nhả khoá.
    pub async fn release(self) -> Result<(), MigrationError> {
        match self.held {
            Held::Postgres { transaction, connection } => {
                // Rollback kết thúc transaction, Postgres nhả advisory lock theo transaction.
                let rolled_back = transaction.rollback().await;
                let closed = connection.close().await;
                rolled_back.and(closed).map_err(|error| {
                    MigrationError(format!("cannot release the migration lock: {error}"))
                })
            }
            Held::File(file) => {
                drop(file);
                Ok(())
            }
            Held::Nothing => Ok(()),
        }
    }
}

/// `data/app.sqlite` → `data/app.sqlite.migrate-lock`.
fn lock_path(database: PathBuf) -> PathBuf {
    let mut path = database.into_os_string();
    path.push(LOCK_FILE_SUFFIX);
    PathBuf::from(path)
}

fn timed_out(wait: Duration) -> MigrationError {
    MigrationError(format!(
        "another process has been applying migrations for more than {} seconds; wait for it to \
         finish (or stop it) and start again",
        wait.as_secs()
    ))
}

async fn acquire_postgres(config: &DatabaseConfig, wait: Duration) -> Result<Held, MigrationError> {
    let connection = crate::database::dedicated_connection(config)
        .await
        .map_err(|error| MigrationError(format!("cannot take the migration lock: {error}")))?;
    let failed = |error: sea_orm::DbErr| {
        MigrationError(format!("cannot take the migration lock: database: {error}"))
    };
    let transaction = connection.begin().await.map_err(failed)?;
    let started = Instant::now();
    let mut logged = started;
    loop {
        let statement = Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT pg_try_advisory_xact_lock($1) AS locked",
            [POSTGRES_KEY.into()],
        );
        let row = transaction.query_one_raw(statement).await.map_err(failed)?;
        let locked = match row {
            Some(row) => row.try_get::<bool>("", "locked").map_err(failed)?,
            None => false,
        };
        if locked {
            return Ok(Held::Postgres { transaction, connection });
        }
        wait_turn(started, &mut logged, wait).await?;
    }
}

async fn acquire_file(path: PathBuf, wait: Duration) -> Result<Held, MigrationError> {
    let cannot = |error: std::io::Error| {
        MigrationError(format!("cannot take the migration lock `{}`: {error}", path.display()))
    };
    if let Some(directory) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        // Gọi hệ thống file kiểu đồng bộ: chỉ một lần lúc khởi động, như `Database::connect`.
        std::fs::create_dir_all(directory).map_err(cannot)?;
    }
    // `truncate(false)`: file chỉ để khoá, nội dung không dùng; không xoá gì của ai.
    let file =
        File::options().create(true).truncate(false).write(true).open(&path).map_err(cannot)?;
    let started = Instant::now();
    let mut logged = started;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Held::File(file)),
            Err(TryLockError::WouldBlock) => wait_turn(started, &mut logged, wait).await?,
            Err(TryLockError::Error(error)) => return Err(cannot(error)),
        }
    }
}

/// Ngủ một lượt trước lần thử sau; quá `wait` thì lỗi. Cứ [`LOG_EVERY`] ghi một dòng log để người
/// vận hành biết tiến trình đang chờ chứ không treo.
async fn wait_turn(
    started: Instant,
    logged: &mut Instant,
    wait: Duration,
) -> Result<(), MigrationError> {
    if started.elapsed() >= wait {
        return Err(timed_out(wait));
    }
    if logged.elapsed() >= LOG_EVERY {
        *logged = Instant::now();
        tracing::info!(
            waited_seconds = started.elapsed().as_secs(),
            "another process is applying migrations; waiting for it to finish"
        );
    }
    tokio::time::sleep(RETRY_EVERY).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_khoa_nam_canh_file_database() {
        assert_eq!(
            lock_path(PathBuf::from("data/app.sqlite")),
            PathBuf::from("data/app.sqlite.migrate-lock")
        );
    }

    #[test]
    fn so_khoa_postgres_co_dinh() {
        assert_eq!(POSTGRES_KEY, 0x6175_746f_7869_6465);
    }
}
