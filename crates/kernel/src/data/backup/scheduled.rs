//! Scheduled backups (step 39, A1): the `kernel.backup` job of the `kernel` module creates a backup for
//! EVERY database this server runs the job for (the main database and each placement's own database), through
//! the same path as `autoxide backup` ([`write_backup`]), writes it to the shared file store under
//! `kernel/backups/<database>/<id>/`, then keeps only the newest `AUTOXIDE_BACKUP_KEEP` good backups per
//! database.
//!
//! The ledger of backup runs is the control-plane table `kernel_backups` (main database). The `active_key`
//! column holds the database name while a run is in progress and `NULL` when done; a unique index on it
//! prevents two concurrent runs for one database even with many workers (multiple `NULL`s are allowed on both
//! SQLite and Postgres). A `running` run whose `heartbeat_at` is older than [`STALE_AFTER`] (the process
//! died) is marked `failed` before a new run is opened.

use crate::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::migration::prelude::{
    DbErr, Index, MigrationName, MigrationTrait, SchemaManager, Table,
};
use crate::migration::schema;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Order, Query};
use sea_orm::{ConnectionTrait, SqlErr};
use uuid::Uuid;

use super::{BACKUP_PREFIX, BackupSink, StorageSink, write_backup};
use crate::config::{ConfigReader, Invalid, Setting, parse_in_range};
use crate::database::Database;
use crate::error::Error;
use crate::job::{JobContext, JobHandler, Schedule};
use crate::storage::{ByteStream, Storage};
use crate::tenancy::DatabaseRegistry;

/// Job kind (and schedule name).
pub const BACKUP_JOB: &str = "kernel.backup";
/// Maximum run time of one run (any database).
pub const BACKUP_TIMEOUT: Duration = Duration::from_secs(6 * 3600);
/// A `running` run with no heartbeat for this long: the process has died.
pub const STALE_AFTER: SignedDuration = SignedDuration::from_mins(15);
/// Interval between `heartbeat_at` updates while running.
const HEARTBEAT_EVERY: Duration = Duration::from_secs(60);
/// Maximum length of the stored error (like a job's `last_error`).
const MAX_ERROR_LENGTH: usize = 2000;
const TABLE: &str = "kernel_backups";

pub const INTERVAL_HOURS: Setting = Setting::new(
    "AUTOXIDE_BACKUP_INTERVAL_HOURS",
    "Back up every database automatically every this many hours (1 to 720) into the file storage \
     (module `files`, a directory or S3) under `kernel/backups/`; `0` turns scheduled backups \
     off. `autoxide doctor` warns when the newest backup is older than twice this.",
)
.with_default("24");

pub const KEEP: Setting = Setting::new(
    "AUTOXIDE_BACKUP_KEEP",
    "Scheduled backups kept per database (1 to 365); older ones are deleted. Data erased on \
     request stays in a backup until that backup is deleted.",
)
.with_default("7");

pub const INCLUDE_FILES: Setting = Setting::new(
    "AUTOXIDE_BACKUP_INCLUDE_FILES",
    "Also copy the stored files (uploads) into each scheduled backup (`true` or `false`). Off by \
     default: the backup goes to that same storage, so copying the files there doubles the space \
     without protecting them — copy the bucket or directory somewhere else instead.",
)
.with_default("false");

/// Scheduled backup configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BackupConfig {
    /// Hours between two runs; 0: disabled.
    pub interval_hours: u64,
    pub keep: u64,
    pub include_files: bool,
}

impl Default for BackupConfig {
    fn default() -> Self {
        BackupConfig { interval_hours: 24, keep: 7, include_files: false }
    }
}

impl BackupConfig {
    pub fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        let interval = reader.get_with(&INTERVAL_HOURS, |raw| {
            let hours: u64 = raw.trim().parse().map_err(|_| "must be a whole number".to_owned())?;
            if hours == 0 { Ok(0) } else { parse_in_range(raw, 1u64, 720) }
        });
        let keep = reader.get_with(&KEEP, |raw| parse_in_range(raw, 1u64, 365));
        let include_files = reader.get_with(&INCLUDE_FILES, |raw| match raw {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("must be `true` or `false`".to_owned()),
        });
        Ok(BackupConfig { interval_hours: interval?, keep: keep?, include_files: include_files? })
    }

    /// Schedule of the job, when enabled.
    pub fn schedule(&self) -> Option<Schedule> {
        (self.interval_hours > 0).then(|| {
            Schedule::new(BACKUP_JOB, BACKUP_JOB, Duration::from_secs(self.interval_hours * 3600))
                .on_control_only()
        })
    }
}

/// The `kernel_backups` table (control plane).
pub(crate) struct CreateBackups;

impl MigrationName for CreateBackups {
    fn name(&self) -> &str {
        "m0036_create_backups"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateBackups {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(TABLE)
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("database_key", 64))
                    .col(schema::string_len("status", 16))
                    .col(schema::string_len_null("active_key", 80))
                    .col(schema::string_len("storage_prefix", 255))
                    .col(schema::timestamp_with_time_zone("started_at"))
                    .col(schema::timestamp_with_time_zone("heartbeat_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .col(schema::big_integer_null("size_bytes"))
                    .col(schema::big_integer_null("rows"))
                    .col(schema::big_integer_null("files"))
                    .col(schema::integer_null("tables"))
                    .col(schema::string_len("autoxide_version", 32))
                    .col(schema::text_null("error"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_backups_active_key")
                    .table(TABLE)
                    .col("active_key")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_backups_database_started")
                    .table(TABLE)
                    .col("database_key")
                    .col("started_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(TABLE).to_owned()).await
    }
}

/// Result of one run for one database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupOutcome {
    Succeeded {
        id: Uuid,
        size_bytes: u64,
    },
    Failed {
        id: Uuid,
        error: String,
    },
    /// Another run is already in progress for this database.
    AlreadyRunning,
}

/// Scheduled backups of all databases.
pub struct ScheduledBackups {
    registry: DatabaseRegistry,
    /// Names of all modules and migrators, available once every module has started.
    setup: crate::tenancy::moves::MoveSetup,
    config: BackupConfig,
}

impl std::fmt::Debug for ScheduledBackups {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduledBackups").field("config", &self.config).finish_non_exhaustive()
    }
}

impl ScheduledBackups {
    pub(crate) fn new(
        registry: DatabaseRegistry,
        setup: crate::tenancy::moves::MoveSetup,
        config: BackupConfig,
    ) -> Self {
        ScheduledBackups { registry, setup, config }
    }

    /// One round: backs up every database, one ledger row per database. Returns the result by database name.
    /// A general failure (no file store, module not started) is an `Err`.
    pub async fn run(&self) -> Result<Vec<(String, BackupOutcome)>, Error> {
        let storage = self.registry.shared_storage().ok_or_else(|| {
            Error::internal(
                "scheduled backups need a file storage (module `files`): install it, or set \
                 AUTOXIDE_BACKUP_INTERVAL_HOURS=0",
            )
        })?;
        let (modules, _) = self.setup.tables()?;
        let placements = self.setup.placements()?;
        let migrator = placements.migrator();
        let control = self.registry.control().clone();
        let mut outcomes = Vec::new();
        for (_, name, database) in self.registry.databases().await? {
            let outcome = self
                .back_up(&control, &name, &database, &modules, migrator, storage.clone())
                .await?;
            outcomes.push((name, outcome));
        }
        Ok(outcomes)
    }

    async fn back_up(
        &self,
        control: &Database,
        name: &str,
        database: &Database,
        modules: &[&'static str],
        migrator: &crate::migration::Migrator,
        storage: Arc<dyn Storage>,
    ) -> Result<BackupOutcome, Error> {
        let Some(id) = open(control, name).await? else {
            tracing::warn!(database = name, "a backup of this database is already running");
            return Ok(BackupOutcome::AlreadyRunning);
        };
        let prefix = prefix_of(name, id);
        let started = Instant::now();
        let sink = CountingSink {
            inner: StorageSink::new(storage.clone(), prefix.clone()),
            bytes: AtomicU64::new(0),
        };
        let scratch = std::env::temp_dir().join(format!("autoxide-backup-{id}"));
        let files = self.config.include_files.then(|| storage.clone());
        let work = write_backup(database, migrator, modules, files, &sink, &scratch);
        // Nhịp `heartbeat_at` chạy song song: lượt còn sống thì không bị coi là chết.
        let result = tokio::select! {
            result = work => result,
            () = heartbeat(control, id) => Err(Error::internal("the heartbeat stopped")),
        };
        let elapsed = started.elapsed().as_secs_f64();
        let bytes = sink.bytes.load(Ordering::Relaxed);
        match result {
            Ok(manifest) => {
                let finished = Timestamp::now();
                let update = Query::update()
                    .table(Alias::new(TABLE))
                    .value(Alias::new("status"), "succeeded")
                    .value(Alias::new("active_key"), Option::<String>::None)
                    .value(Alias::new("finished_at"), finished)
                    .value(Alias::new("size_bytes"), i64::try_from(bytes).unwrap_or(i64::MAX))
                    .value(Alias::new("rows"), i64::try_from(manifest.rows()).unwrap_or(i64::MAX))
                    .value(Alias::new("files"), i64::try_from(manifest.files).unwrap_or(i64::MAX))
                    .value(
                        Alias::new("tables"),
                        i32::try_from(manifest.tables.len()).unwrap_or(i32::MAX),
                    )
                    .and_where(Expr::col(Alias::new("id")).eq(id))
                    .to_owned();
                control.write().execute(&update).await?;
                record_metrics(name, true, elapsed, Some((bytes, finished)));
                tracing::info!(database = name, backup_id = %id, size_bytes = bytes, "backup done");
                if let Err(error) = self.prune(control, name, storage.as_ref()).await {
                    tracing::warn!(database = name, %error, "could not delete old backups");
                }
                Ok(BackupOutcome::Succeeded { id, size_bytes: bytes })
            }
            Err(error) => {
                let message = describe(&error);
                fail(control, id, &message).await?;
                record_metrics(name, false, elapsed, None);
                tracing::error!(database = name, backup_id = %id, error = %message, "backup failed");
                // Phần đã ghi của bản hỏng: dọn luôn, không để nằm lại chiếm chỗ.
                remove_files(storage.as_ref(), &prefix).await;
                Ok(BackupOutcome::Failed { id, error: message })
            }
        }
    }

    /// Keeps only the newest `keep` `succeeded` backups of `name`; deletes the files and rows of older
    /// ones, plus every `failed` run older than the oldest retained one. Always keeps the newest good
    /// backup.
    async fn prune(
        &self,
        control: &Database,
        name: &str,
        storage: &dyn Storage,
    ) -> Result<(), Error> {
        let select = Query::select()
            .columns([Alias::new("id"), Alias::new("status"), Alias::new("storage_prefix")])
            .from(Alias::new(TABLE))
            .and_where(Expr::col(Alias::new("database_key")).eq(name))
            .and_where(Expr::col(Alias::new("active_key")).is_null())
            .order_by(Alias::new("started_at"), Order::Desc)
            .order_by(Alias::new("id"), Order::Desc)
            .to_owned();
        let rows = control.read().query_all(&select).await?;
        let mut kept = 0u64;
        for row in rows {
            let id: Uuid = row.try_get("", "id")?;
            let status: String = row.try_get("", "status")?;
            let prefix: String = row.try_get("", "storage_prefix")?;
            if status == "succeeded" && kept < self.config.keep {
                kept += 1;
                continue;
            }
            // Bản `failed` mới hơn bản tốt được giữ thì để lại cho người vận hành xem lỗi.
            if status != "succeeded" && kept < self.config.keep {
                continue;
            }
            remove_files(storage, &prefix).await;
            let delete = Query::delete()
                .from_table(Alias::new(TABLE))
                .and_where(Expr::col(Alias::new("id")).eq(id))
                .to_owned();
            control.write().execute(&delete).await?;
        }
        Ok(())
    }
}

/// Storage prefix of one backup.
pub fn prefix_of(database: &str, id: Uuid) -> String {
    format!("{BACKUP_PREFIX}{database}/{id}/")
}

/// Opens a `running` run for `name`; `None` when another run is in progress. A dead run (heartbeat too old)
/// is marked `failed` first.
async fn open(control: &Database, name: &str) -> Result<Option<Uuid>, Error> {
    let now = Timestamp::now();
    let stale = Query::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("status"), "failed")
        .value(Alias::new("active_key"), Option::<String>::None)
        .value(Alias::new("finished_at"), now)
        .value(Alias::new("error"), "the process making this backup stopped before it finished")
        .and_where(Expr::col(Alias::new("active_key")).eq(name))
        .and_where(Expr::col(Alias::new("heartbeat_at")).lt(now - STALE_AFTER))
        .to_owned();
    if control.write().execute(&stale).await?.rows_affected() > 0 {
        metrics::counter!("backup_runs_total", "outcome" => "failed").increment(1);
    }
    let id = Uuid::now_v7();
    let insert = Query::insert()
        .into_table(Alias::new(TABLE))
        .columns(
            [
                "id",
                "database_key",
                "status",
                "active_key",
                "storage_prefix",
                "started_at",
                "heartbeat_at",
                "autoxide_version",
            ]
            .map(Alias::new),
        )
        .values_panic([
            id.into(),
            name.into(),
            "running".into(),
            name.into(),
            prefix_of(name, id).into(),
            now.into(),
            now.into(),
            env!("CARGO_PKG_VERSION").into(),
        ])
        .to_owned();
    match control.write().execute(&insert).await {
        Ok(_) => Ok(Some(id)),
        Err(error) if matches!(error.sql_err(), Some(SqlErr::UniqueConstraintViolation(_))) => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Updates `heartbeat_at` every [`HEARTBEAT_EVERY`]; returns only when the row is no longer `running` (someone
/// marked it failed), in which case this run must stop.
async fn heartbeat(control: &Database, id: Uuid) {
    loop {
        tokio::time::sleep(HEARTBEAT_EVERY).await;
        let update = Query::update()
            .table(Alias::new(TABLE))
            .value(Alias::new("heartbeat_at"), Timestamp::now())
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .and_where(Expr::col(Alias::new("status")).eq("running"))
            .to_owned();
        match control.write().execute(&update).await {
            Ok(result) if result.rows_affected() == 0 => return,
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(backup_id = %id, %error, "cannot update a backup heartbeat")
            }
        }
    }
}

async fn fail(control: &Database, id: Uuid, message: &str) -> Result<(), Error> {
    let message: String = message.chars().take(MAX_ERROR_LENGTH).collect();
    let update = Query::update()
        .table(Alias::new(TABLE))
        .value(Alias::new("status"), "failed")
        .value(Alias::new("active_key"), Option::<String>::None)
        .value(Alias::new("finished_at"), Timestamp::now())
        .value(Alias::new("error"), message)
        .and_where(Expr::col(Alias::new("id")).eq(id))
        .to_owned();
    control.write().execute(&update).await?;
    Ok(())
}

/// Deletes every file under `prefix`; failures are only logged (the next cleanup retries).
async fn remove_files(storage: &dyn Storage, prefix: &str) {
    let keys = match storage.keys_with_prefix(prefix).await {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(prefix, %error, "cannot list the files of a backup");
            return;
        }
    };
    for key in keys {
        if let Err(error) = storage.delete(&key).await {
            tracing::warn!(key, %error, "cannot delete a backup file");
        }
    }
}

#[allow(clippy::cast_precision_loss)] // Gauge là f64; số byte và giây không tới 2^53.
fn record_metrics(
    database: &str,
    succeeded: bool,
    seconds: f64,
    success: Option<(u64, Timestamp)>,
) {
    let outcome = if succeeded { "succeeded" } else { "failed" };
    metrics::counter!("backup_runs_total", "outcome" => outcome).increment(1);
    let label = [("database", database.to_owned())];
    metrics::histogram!("backup_duration_seconds", &label).record(seconds);
    if let Some((bytes, at)) = success {
        metrics::gauge!("backup_size_bytes", &label).set(bytes as f64);
        metrics::gauge!("backup_last_success_timestamp_seconds", &label)
            .set(at.as_unix_seconds() as f64);
    }
}

fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.message()),
        None => error.message(),
    }
}

/// Wraps a [`BackupSink`] to count the bytes written (backup size).
struct CountingSink<S> {
    inner: S,
    bytes: AtomicU64,
}

#[async_trait::async_trait]
impl<S: BackupSink> BackupSink for CountingSink<S> {
    async fn put(&self, relative: &str, content: ByteStream) -> Result<u64, Error> {
        let written = self.inner.put(relative, content).await?;
        self.bytes.fetch_add(written, Ordering::Relaxed);
        Ok(written)
    }
}

/// Runner of the `kernel.backup` job.
pub(crate) struct BackupRunner(pub Arc<ScheduledBackups>);

#[async_trait::async_trait]
impl JobHandler for BackupRunner {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        let outcomes = self.0.run().await?;
        // Một database hỏng: job vẫn xong (lượt sau thử lại theo lịch); lỗi nằm ở sổ, log, số đo.
        // Không thử lại ngay: một lỗi kéo dài (đầy đĩa) sẽ lặp lại mỗi lần thử.
        let failed: Vec<&str> = outcomes
            .iter()
            .filter(|(_, outcome)| matches!(outcome, BackupOutcome::Failed { .. }))
            .map(|(name, _)| name.as_str())
            .collect();
        if !failed.is_empty() {
            tracing::error!(databases = failed.join(","), "scheduled backup failed");
        }
        Ok(())
    }
}

/// Newest good backup on every database: `(database name, finish time)`, oldest first, for
/// `autoxide doctor`.
pub async fn latest_successes(control: &Database) -> Result<Vec<(String, Timestamp)>, Error> {
    let select = Query::select()
        .columns([Alias::new("database_key"), Alias::new("finished_at")])
        .from(Alias::new(TABLE))
        .and_where(Expr::col(Alias::new("status")).eq("succeeded"))
        .order_by(Alias::new("finished_at"), Order::Desc)
        .to_owned();
    let mut newest: Vec<(String, Timestamp)> = Vec::new();
    for row in control.read().query_all(&select).await? {
        let name: String = row.try_get("", "database_key")?;
        let at: Timestamp = row.try_get("", "finished_at")?;
        if !newest.iter().any(|(known, _)| *known == name) {
            newest.push((name, at));
        }
    }
    newest.sort_by_key(|(_, at)| *at);
    Ok(newest)
}

/// The `kernel.backup_age` check step of `autoxide doctor`.
pub(crate) struct BackupAge {
    pub registry: DatabaseRegistry,
    pub config: BackupConfig,
}

#[async_trait::async_trait]
impl crate::doctor::Probe for BackupAge {
    async fn probe(&self) -> crate::doctor::Finding {
        use crate::doctor::Finding;
        if self.config.interval_hours == 0 {
            return Finding::warning(
                "Scheduled backups are off (AUTOXIDE_BACKUP_INTERVAL_HOURS=0): nothing backs the \
                 data up automatically.",
            )
            .with_hint(
                "Set AUTOXIDE_BACKUP_INTERVAL_HOURS (24 is daily), or run `autoxide backup` from \
                 your own scheduler and keep the copies away from this server.",
            );
        }
        if self.registry.shared_storage().is_none() {
            return Finding::warning(
                "Scheduled backups have nowhere to go: no file storage is configured (module \
                 `files`).",
            )
            .with_hint(
                "Install the `files` module with a directory or S3 bucket, or run `autoxide \
                 backup` from your own scheduler and keep the copies away from this server.",
            );
        }
        let latest = match latest_successes(self.registry.control()).await {
            Ok(latest) => latest,
            Err(error) => return Finding::failed(format!("cannot read kernel_backups: {error}")),
        };
        let databases = match self.registry.databases().await {
            Ok(databases) => databases,
            Err(error) => return Finding::failed(format!("cannot list the databases: {error}")),
        };
        let limit = SignedDuration::from_hours(
            i64::try_from(self.config.interval_hours.saturating_mul(2)).unwrap_or(i64::MAX),
        );
        let now = Timestamp::now();
        let mut old = Vec::new();
        for (_, name, _) in &databases {
            match latest.iter().find(|(known, _)| known == name) {
                Some((_, at)) if now - *at <= limit => {}
                Some((_, at)) => {
                    old.push(format!("{name} (last {})", at.format("%Y-%m-%d %H:%M UTC")))
                }
                None => old.push(format!("{name} (none yet)")),
            }
        }
        if old.is_empty() {
            return Finding::ok(format!(
                "Every database has a scheduled backup from the last {} hours.",
                self.config.interval_hours * 2
            ));
        }
        Finding::warning(format!(
            "No scheduled backup in the last {} hours for: {}.",
            self.config.interval_hours * 2,
            old.join(", ")
        ))
        .with_hint(
            "Check that a process with AUTOXIDE_PROCESS_ROLE=all or worker runs, and read the \
             `error` of the newest row of `kernel_backups` (or the log line `backup failed`); a \
             new installation gets its first backup within AUTOXIDE_BACKUP_INTERVAL_HOURS.",
        )
    }
}
