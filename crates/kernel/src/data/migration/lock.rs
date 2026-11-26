//! Migration lock: only ONE process runs a database's migrations at a time.
//!
//! Needed when the server migrates itself on startup (`AUTOXIDE_MIGRATE_ON_START`,
//! D-20261121-migrate-on-start): when several server instances start at once, the first one migrates and the
//! others wait for it to finish, then find nothing left to do. `autoxide migrate up` takes this lock too.
//!
//! The two databases have no common mechanism, so the code branches here (behind a single type,
//! [`MigrationLock`]) and tests run on both:
//!
//! - **Postgres**: a transaction-scoped advisory lock (`pg_try_advisory_xact_lock`) on a dedicated connection;
//!   the transaction stays open until the lock is released. If the process dies the connection drops and
//!   Postgres releases the lock itself, so there is no "orphan" lock.
//! - **SQLite**: the database is a local file, so the lock is an operating-system file lock
//!   (`File::try_lock`) on the file `<database>.migrate-lock` next to it. If the process dies the operating
//!   system releases it. An in-memory SQLite database is visible to one process only: no lock is needed.

use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseTransaction, DbBackend, TransactionTrait,
};

use super::MigrationError;
use crate::config::{DatabaseBackend, DatabaseConfig};

/// Advisory lock identifier of the migration on Postgres: the string "autoxide" (8 ASCII bytes) read as a
/// 64-bit integer. Fixed: every Autoxide version uses this same number.
const POSTGRES_KEY: i64 = i64::from_be_bytes(*b"autoxide");

/// Suffix appended to the SQLite file path to form the lock file.
const LOCK_FILE_SUFFIX: &str = ".migrate-lock";

/// Interval between two attempts to take the lock while another process holds it.
const RETRY_EVERY: Duration = Duration::from_millis(250);

/// Interval between two "waiting" log lines.
const LOG_EVERY: Duration = Duration::from_secs(10);

/// The migration lock being held. Release it with [`release`](Self::release); dropping it also releases it
/// (the connection or file closes), only without waiting for Postgres to confirm.
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
    /// In-memory SQLite.
    Nothing,
}

impl MigrationLock {
    /// Takes the migration lock of database `config`. If another process holds it, waits and retries every
    /// 250 ms, up to `wait`; on timeout returns an error that says what to do.
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

    /// A lock for an installation with nothing to share it with (a registry built without the database
    /// configuration, in-memory SQLite): holds nothing.
    pub(crate) fn unshared() -> Self {
        MigrationLock { held: Held::Nothing }
    }

    /// Releases the lock.
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
        let statement = crate::database::statement(
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

/// Sleeps before the next attempt; past `wait` it is an error. Every [`LOG_EVERY`] it logs a line so the
/// operator knows the process is waiting and not hung.
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
