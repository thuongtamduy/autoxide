//! Backlog measurements (step 39, item A3): pending jobs, the age of the longest-waiting due job, jobs in the
//! dead-letter queue, undelivered events and the age of the oldest event, database connections in use / idle
//! — per database and module. Cumulative counters (`jobs_failed_total`…) do not show that a queue is STUCK;
//! these gauges do.
//!
//! Two parts kept separate so they can be tested on both databases without touching the process's metrics
//! recorder: [`backlog`] only reads the database and returns a [`BacklogSnapshot`]; [`publish`] only sets
//! gauges. [`BacklogMonitor`] runs both periodically in the process that runs jobs (role `worker` / `all`).
//!
//! Labels: `database` (the placement name, capped by `AUTOXIDE_METRICS_MAX_DATABASE_LABELS` - the excess is
//! merged into `other`), `module` (registered modules: a small fixed set), `pool`, `state`. NO per-tenant
//! label: per-tenant lookup goes through logs, traces and the CLI (the code rule on metric labels).

use crate::time::Timestamp;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Func, Order, Query};
use std::time::Duration;

use crate::config::DatabaseBackend;

use crate::database::Database;
use crate::error::Error;
use crate::tenancy::DatabaseRegistry;

/// The `database` label for databases beyond the limit.
pub const OTHER_DATABASE: &str = "other";

/// Names of every gauge [`publish`] sets. The dashboard tests and the alert rules in `deploy/` compare metric
/// names against this list (along with the kernel's other metrics).
pub const METRICS: &[&str] = &[
    "jobs_pending",
    "jobs_oldest_pending_seconds",
    "jobs_dead",
    "events_pending",
    "events_oldest_pending_seconds",
    "events_dead",
    "database_connections",
    "database_connections_max",
];

/// The backlog of one database.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BacklogSnapshot {
    /// Database (placement) name: `main`, or a step 33 placement name.
    pub database: String,
    pub modules: Vec<ModuleBacklog>,
    pub pools: Vec<PoolUsage>,
}

/// The backlog of one module on one database. A module without a job table (or event table) has the
/// corresponding numbers as 0.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModuleBacklog {
    pub module: &'static str,
    /// Jobs not yet done and not in the dead-letter queue (including scheduled jobs not yet due).
    pub jobs_pending: u64,
    /// Age (seconds) of the longest-waiting DUE job: `now - run_at`; 0 when there is none. A job scheduled
    /// for the future does not count - it is not waiting on anyone yet.
    pub jobs_oldest_pending_seconds: u64,
    /// Jobs in the dead-letter queue (`failed_at` has a value).
    pub jobs_dead: u64,
    /// Events not yet delivered and not permanently failed.
    pub events_pending: u64,
    /// Age (seconds) of the oldest undelivered event: `now - occurred_at`; 0 when there is none.
    pub events_oldest_pending_seconds: u64,
    /// Permanently failed events (`failed_at` has a value).
    pub events_dead: u64,
}

/// The connections of one pool.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolUsage {
    /// `read` hay `write`.
    pub pool: &'static str,
    pub in_use: u32,
    pub idle: u32,
    pub max: u32,
}

/// Reads the backlog of `database` (named `name`): the job tables of `job_modules`, the event tables of
/// `event_modules`, and the two pools. Uses only the READ connection (SQLite has only one write connection).
pub async fn backlog(
    database: &Database,
    name: &str,
    job_modules: &[&'static str],
    event_modules: &[&'static str],
    now: Timestamp,
) -> Result<BacklogSnapshot, Error> {
    let mut names: Vec<&'static str> = job_modules.iter().chain(event_modules).copied().collect();
    names.sort_unstable();
    names.dedup();
    let mut modules = Vec::with_capacity(names.len());
    for module in names {
        let mut found = ModuleBacklog { module, ..ModuleBacklog::default() };
        if job_modules.contains(&module) {
            let table = super::table_name(module);
            let open = Condition::all()
                .add(Expr::col(Alias::new("completed_at")).is_null())
                .add(Expr::col(Alias::new("failed_at")).is_null());
            found.jobs_pending = count(database, &table, open.clone()).await?;
            let due = open.add(Expr::col(Alias::new("run_at")).lte(now));
            found.jobs_oldest_pending_seconds =
                oldest(database, &table, "run_at", due, now).await?;
            found.jobs_dead =
                count(database, &table, Expr::col(Alias::new("failed_at")).is_not_null().into())
                    .await?;
        }
        if event_modules.contains(&module) {
            let table = crate::event::table_name(module);
            let open = Condition::all()
                .add(Expr::col(Alias::new("delivered_at")).is_null())
                .add(Expr::col(Alias::new("failed_at")).is_null());
            found.events_pending = count(database, &table, open.clone()).await?;
            found.events_oldest_pending_seconds =
                oldest(database, &table, "occurred_at", open, now).await?;
            found.events_dead =
                count(database, &table, Expr::col(Alias::new("failed_at")).is_not_null().into())
                    .await?;
        }
        modules.push(found);
    }
    Ok(BacklogSnapshot { database: name.to_owned(), modules, pools: pools(database) })
}

/// Number of rows of `table` satisfying `condition`.
async fn count(database: &Database, table: &str, condition: Condition) -> Result<u64, Error> {
    let select = Query::select()
        .expr_as(Func::count(Expr::col(Alias::new("id"))), Alias::new("n"))
        .from(Alias::new(table))
        .cond_where(condition)
        .to_owned();
    let row = database.read().query_one(&select).await?;
    let n: i64 = match row {
        Some(row) => row.try_get("", "n")?,
        None => 0,
    };
    Ok(u64::try_from(n).unwrap_or(0))
}

/// Age (seconds, up to `now`) of the row with the smallest `column` among the rows satisfying `condition`; 0
/// when there are no rows. Takes the first row in order instead of `MIN(...)`: the return type of an aggregate
/// over a time column differs between SQLite (text) and Postgres, whereas reading a column is like
/// everywhere else.
async fn oldest(
    database: &Database,
    table: &str,
    column: &str,
    condition: Condition,
    now: Timestamp,
) -> Result<u64, Error> {
    let select = Query::select()
        .column(Alias::new(column))
        .from(Alias::new(table))
        .cond_where(condition)
        .order_by(Alias::new(column), Order::Asc)
        .limit(1)
        .to_owned();
    let Some(row) = database.read().query_one(&select).await? else {
        return Ok(0);
    };
    let at: Timestamp = row.try_get("", column)?;
    Ok(u64::try_from((now - at).as_secs()).unwrap_or(0))
}

/// The connections of the two pools. Read straight from sqlx's pool (no database query).
fn pools(database: &Database) -> Vec<PoolUsage> {
    [("read", database.read()), ("write", database.write())]
        .into_iter()
        .map(|(pool, connection)| {
            let (size, idle, max) = match database.backend() {
                DatabaseBackend::Sqlite => {
                    let p = connection.get_sqlite_connection_pool();
                    (p.size(), p.num_idle(), p.options().get_max_connections())
                }
                DatabaseBackend::Postgres => {
                    let p = connection.get_postgres_connection_pool();
                    (p.size(), p.num_idle(), p.options().get_max_connections())
                }
            };
            let idle = std::cmp::min(u32::try_from(idle).unwrap_or(u32::MAX), size);
            PoolUsage { pool, in_use: size - idle, idle, max }
        })
        .collect()
}

/// Sets the gauges from `snapshots`. Beyond `max_database_labels` databases the rest (in the order of
/// `snapshots`) are merged into the label `database="other"`: counts are summed, the largest age is taken -
/// so the number of metric series has a ceiling even when each tenant has its own database.
pub fn publish(snapshots: &[BacklogSnapshot], max_database_labels: usize) {
    let (named, rest) = snapshots.split_at(snapshots.len().min(max_database_labels));
    for snapshot in named {
        publish_one(&snapshot.database, &snapshot.modules, &snapshot.pools);
    }
    if rest.is_empty() {
        return;
    }
    let mut modules: Vec<ModuleBacklog> = Vec::new();
    let mut pools: Vec<PoolUsage> = Vec::new();
    for snapshot in rest {
        for found in &snapshot.modules {
            match modules.iter_mut().find(|known| known.module == found.module) {
                Some(known) => {
                    known.jobs_pending += found.jobs_pending;
                    known.jobs_oldest_pending_seconds = std::cmp::max(
                        known.jobs_oldest_pending_seconds,
                        found.jobs_oldest_pending_seconds,
                    );
                    known.jobs_dead += found.jobs_dead;
                    known.events_pending += found.events_pending;
                    known.events_oldest_pending_seconds = std::cmp::max(
                        known.events_oldest_pending_seconds,
                        found.events_oldest_pending_seconds,
                    );
                    known.events_dead += found.events_dead;
                }
                None => modules.push(found.clone()),
            }
        }
        for found in &snapshot.pools {
            match pools.iter_mut().find(|known| known.pool == found.pool) {
                Some(known) => {
                    known.in_use += found.in_use;
                    known.idle += found.idle;
                    known.max += found.max;
                }
                None => pools.push(found.clone()),
            }
        }
    }
    publish_one(OTHER_DATABASE, &modules, &pools);
}

#[allow(clippy::cast_precision_loss)] // Gauge là f64; số đếm không bao giờ tới 2^53.
fn publish_one(database: &str, modules: &[ModuleBacklog], pools: &[PoolUsage]) {
    for found in modules {
        let labels = [("database", database.to_owned()), ("module", found.module.to_owned())];
        metrics::gauge!("jobs_pending", &labels).set(found.jobs_pending as f64);
        metrics::gauge!("jobs_oldest_pending_seconds", &labels)
            .set(found.jobs_oldest_pending_seconds as f64);
        metrics::gauge!("jobs_dead", &labels).set(found.jobs_dead as f64);
        metrics::gauge!("events_pending", &labels).set(found.events_pending as f64);
        metrics::gauge!("events_oldest_pending_seconds", &labels)
            .set(found.events_oldest_pending_seconds as f64);
        metrics::gauge!("events_dead", &labels).set(found.events_dead as f64);
    }
    for pool in pools {
        let base = [("database", database.to_owned()), ("pool", pool.pool.to_owned())];
        for (state, value) in [("in_use", pool.in_use), ("idle", pool.idle)] {
            let labels = [base[0].clone(), base[1].clone(), ("state", state.to_owned())];
            metrics::gauge!("database_connections", &labels).set(f64::from(value));
        }
        metrics::gauge!("database_connections_max", &base).set(f64::from(pool.max));
    }
}

/// Periodically updates the backlog gauges for every database this server runs jobs on. Built with
/// `Modules::backlog_monitor`; runs in a process with role `worker` or `all`.
pub struct BacklogMonitor {
    databases: DatabaseRegistry,
    job_modules: Vec<&'static str>,
    event_modules: Vec<&'static str>,
    every: Duration,
    max_database_labels: usize,
}

impl BacklogMonitor {
    pub(crate) fn new(
        databases: DatabaseRegistry,
        job_modules: Vec<&'static str>,
        event_modules: Vec<&'static str>,
    ) -> Self {
        BacklogMonitor {
            databases,
            job_modules,
            event_modules,
            every: Duration::from_secs(30),
            max_database_labels: 50,
        }
    }

    /// Update interval (`AUTOXIDE_METRICS_BACKLOG_SECONDS`).
    pub fn every(mut self, every: Duration) -> Self {
        self.every = every.max(Duration::from_secs(1));
        self
    }

    /// Maximum number of `database` labels (`AUTOXIDE_METRICS_MAX_DATABASE_LABELS`).
    pub fn max_database_labels(mut self, max: usize) -> Self {
        self.max_database_labels = max.max(1);
        self
    }

    /// One round: reads the backlog of every database. A failing database is logged and skipped, without
    /// breaking the round for other databases.
    pub async fn collect(&self) -> Result<Vec<BacklogSnapshot>, Error> {
        let now = Timestamp::now();
        let mut snapshots = Vec::new();
        for (_, name, database) in self.databases.databases().await? {
            match backlog(&database, &name, &self.job_modules, &self.event_modules, now).await {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(error) => {
                    tracing::warn!(database = %name, %error, "could not read the queue backlog");
                }
            }
        }
        Ok(snapshots)
    }

    /// Runs until `shutdown` completes: each interval one [`collect`](Self::collect) round and then
    /// [`publish`].
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        let mut ticks = tokio::time::interval(self.every);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = &mut shutdown => return,
                _ = ticks.tick() => {}
            }
            match self.collect().await {
                Ok(snapshots) => publish(&snapshots, self.max_database_labels),
                Err(error) => tracing::warn!(%error, "could not list databases for the backlog"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(database: &str, pending: u64, oldest: u64) -> BacklogSnapshot {
        BacklogSnapshot {
            database: database.to_owned(),
            modules: vec![ModuleBacklog {
                module: "notes",
                jobs_pending: pending,
                jobs_oldest_pending_seconds: oldest,
                ..ModuleBacklog::default()
            }],
            pools: vec![PoolUsage { pool: "write", in_use: 1, idle: 2, max: 3 }],
        }
    }

    /// Beyond the label limit the remaining databases are merged into `other`: counts are summed, the largest
    /// age is taken. Uses a local metrics recorder so as not to touch the process-wide one.
    #[test]
    fn publish_gop_database_vuot_gioi_han_thanh_other() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        metrics::with_local_recorder(&recorder, || {
            publish(&[snapshot("main", 1, 10), snapshot("b", 2, 30), snapshot("c", 3, 20)], 1);
        });
        let text = handle.render();
        assert!(text.contains(r#"jobs_pending{database="main",module="notes"} 1"#), "{text}");
        assert!(text.contains(r#"jobs_pending{database="other",module="notes"} 5"#), "{text}");
        assert!(
            text.contains(r#"jobs_oldest_pending_seconds{database="other",module="notes"} 30"#),
            "{text}"
        );
        assert!(
            text.contains(r#"database_connections_max{database="other",pool="write"} 6"#),
            "{text}"
        );
        assert!(!text.contains(r#"database="b""#), "{text}");
        for name in METRICS {
            assert!(text.contains(name), "{name} missing: {text}");
        }
    }
}

/// Modules that have a job table and an event table, for [`status`] (REST, MCP). `Modules` fills it in after
/// every module has started; empty before that. Cheap to `Clone` (a reference-counted pointer).
#[derive(Clone, Default)]
pub(crate) struct BacklogModules(std::sync::Arc<std::sync::OnceLock<ModuleLists>>);

type ModuleLists = (Vec<&'static str>, Vec<&'static str>);

impl BacklogModules {
    pub(crate) fn fill(&self, jobs: Vec<&'static str>, events: Vec<&'static str>) {
        let _ = self.0.set((jobs, events));
    }

    fn get(&self) -> Result<&ModuleLists, Error> {
        self.0.get().ok_or_else(|| Error::internal("the backlog is read before modules started"))
    }
}

/// The backlog of every database, with the latest good backup of each database - for
/// `GET /api/v1/kernel/ops/backlog` and the MCP tool `kernel.ops_backlog`.
pub(crate) async fn status(
    registry: &DatabaseRegistry,
    modules: &BacklogModules,
) -> Result<super::backlog_api::OpsBacklog, Error> {
    let (jobs, events) = modules.get()?;
    let now = Timestamp::now();
    let backups = crate::backup::scheduled::latest_successes(registry.control()).await?;
    let mut databases = Vec::new();
    for (_, name, database) in registry.databases().await? {
        let snapshot = backlog(&database, &name, jobs, events, now).await?;
        let last_backup_at = backups.iter().find(|(known, _)| *known == name).map(|(_, at)| *at);
        databases
            .push(super::backlog_api::DatabaseBacklog::from_snapshot(snapshot, last_backup_at));
    }
    Ok(super::backlog_api::OpsBacklog { checked_at: now, databases })
}
