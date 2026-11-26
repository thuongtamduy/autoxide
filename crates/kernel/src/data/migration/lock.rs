//! Migration lock: only ONE process runs a database's migrations at a time.
//!
//! Needed when the server migrates itself on startup (`AUTOXIDE_MIGRATE_ON_START`,
//! D-20261121-migrate-on-start): when several server instances start at once, the first one migrates and the
//! others wait for it to finish, then find nothing left to do. `autoxide migrate up` takes this lock too.
//!
//! The two databases have no common mechanism, so the code branches here (behind a single type,
//! [`MigrationLock`]) and tests run on both:
//!
//! - **Postgres**: a session-level advisory lock (`pg_try_advisory_lock`) on one connection held out of its
//!   pool for as long as the lock (no open transaction, so `idle_in_transaction_session_timeout` does not
//!   apply). If the process dies the connection drops and Postgres releases the lock itself, so there is no
//!   "orphan" lock. A proxy or failover that cuts the connection releases it too: a long holder calls
//!   [`MigrationLock::verify`] before each step and stops when the lock is gone.
//! - **SQLite**: the database is a local file, so the lock is an operating-system file lock
//!   (`File::try_lock`) on the file `<database>.migrate-lock` next to it. If the process dies the operating
//!   system releases it. An in-memory SQLite database is visible to one process only: no lock is needed.
//!
//! Two locks (D-20261126-placement-migration-exclusive): the main one ([`MigrationLock::acquire`]) for the
//! control database, and [`MigrationLock::acquire_placements`] for the tenant databases (placements), so a long
//! background run over the placements never holds up a server that starts and only migrates the main
//! database. Whoever needs both takes the main one first.

use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sea_orm::DatabaseConnection;
use sea_orm::sqlx::Postgres;
use sea_orm::sqlx::pool::PoolConnection;

use super::MigrationError;
use crate::config::{DatabaseBackend, DatabaseConfig};

/// Advisory lock identifier of the migration on Postgres: the string "autoxide" (8 ASCII bytes) read as a
/// 64-bit integer. Fixed: every Autoxide version uses this same number.
const POSTGRES_KEY: i64 = i64::from_be_bytes(*b"autoxide");
/// Advisory lock identifier of the placement migrations: "autoxplc".
const POSTGRES_PLACEMENTS_KEY: i64 = i64::from_be_bytes(*b"autoxplc");

/// Suffix appended to the SQLite file path to form the lock file.
const LOCK_FILE_SUFFIX: &str = ".migrate-lock";
/// Suffix of the lock file of the placement migrations.
const PLACEMENTS_LOCK_FILE_SUFFIX: &str = ".placements-lock";

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
    // Lấy hẳn một kết nối ra khỏi pool riêng và giữ nó: kết nối đang mượn thì pool không đóng vì rỗi hay quá
    // tuổi, nên advisory lock theo session không mất ngầm. Pool phải sống tới khi trả kết nối.
    Postgres {
        connection: PoolConnection<Postgres>,
        pool: DatabaseConnection,
        key: i64,
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
        Self::take(config, wait, POSTGRES_KEY, LOCK_FILE_SUFFIX).await
    }

    /// Takes the lock of the placement migrations (tenant databases) of the installation whose control database
    /// is `config`; like [`acquire`](Self::acquire). Independent of the main lock: a server starting while a
    /// background run migrates the placements does not wait for it.
    pub async fn acquire_placements(
        config: &DatabaseConfig,
        wait: Duration,
    ) -> Result<Self, MigrationError> {
        Self::take(config, wait, POSTGRES_PLACEMENTS_KEY, PLACEMENTS_LOCK_FILE_SUFFIX).await
    }

    async fn take(
        config: &DatabaseConfig,
        wait: Duration,
        key: i64,
        suffix: &str,
    ) -> Result<Self, MigrationError> {
        let held = match config.url.backend() {
            DatabaseBackend::Postgres => acquire_postgres(config, wait, key).await?,
            DatabaseBackend::Sqlite => match config.url.sqlite_path() {
                Some(path) => acquire_file(lock_path(path, suffix), wait).await?,
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

    /// Checks that the lock is still held: on Postgres a dropped connection (proxy, failover, server restart)
    /// releases it without telling anyone, so a long holder checks before each step and stops on an error.
    pub async fn verify(&mut self) -> Result<(), MigrationError> {
        let Held::Postgres { connection, key, .. } = &mut self.held else {
            return Ok(());
        };
        // Khoá bigint nằm trong pg_locks dưới dạng hai nửa 32 bit: classid (nửa cao), objid (nửa thấp), objsubid 1.
        let high = i64::from((*key >> 32) as u32);
        let low = i64::from(*key as u32);
        let held = sea_orm::sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid() \
             AND classid::bigint = $1 AND objid::bigint = $2 AND objsubid = 1 AND granted)",
        )
        .bind(high)
        .bind(low)
        .fetch_one(&mut **connection)
        .await
        .unwrap_or(false);
        if held {
            Ok(())
        } else {
            Err(MigrationError(
                "the migration lock was lost (the database connection dropped); stop and run again"
                    .to_owned(),
            ))
        }
    }

    /// Releases the lock.
    pub async fn release(self) -> Result<(), MigrationError> {
        match self.held {
            Held::Postgres { mut connection, pool, key } => {
                let unlocked = sea_orm::sqlx::query("SELECT pg_advisory_unlock($1)")
                    .bind(key)
                    .execute(&mut *connection)
                    .await
                    .map(drop);
                drop(connection);
                let closed = pool.close().await.map_err(|error| error.to_string());
                unlocked.map_err(|error| error.to_string()).and(closed).map_err(|error| {
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

/// `data/app.sqlite` → `data/app.sqlite.migrate-lock` (or `.placements-lock`).
fn lock_path(database: PathBuf, suffix: &str) -> PathBuf {
    let mut path = database.into_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn timed_out(wait: Duration) -> MigrationError {
    MigrationError(format!(
        "another process has been applying migrations for more than {} seconds; wait for it to \
         finish (or stop it) and start again",
        wait.as_secs()
    ))
}

async fn acquire_postgres(
    config: &DatabaseConfig,
    wait: Duration,
    key: i64,
) -> Result<Held, MigrationError> {
    let pool = crate::database::dedicated_connection(config)
        .await
        .map_err(|error| MigrationError(format!("cannot take the migration lock: {error}")))?;
    let failed = |error: sea_orm::sqlx::Error| {
        MigrationError(format!("cannot take the migration lock: database: {error}"))
    };
    let mut connection = pool.get_postgres_connection_pool().acquire().await.map_err(failed)?;
    let started = Instant::now();
    let mut logged = started;
    loop {
        let locked = sea_orm::sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(&mut *connection)
            .await
            .map_err(failed)?;
        if locked {
            return Ok(Held::Postgres { connection, pool, key });
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
            lock_path(PathBuf::from("data/app.sqlite"), LOCK_FILE_SUFFIX),
            PathBuf::from("data/app.sqlite.migrate-lock")
        );
    }

    #[test]
    fn so_khoa_postgres_co_dinh() {
        assert_eq!(POSTGRES_KEY, 0x6175_746f_7869_6465);
    }
}
