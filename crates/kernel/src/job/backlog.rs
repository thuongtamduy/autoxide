//! Số đo tồn đọng (bước 39, mục A3): job đang chờ, tuổi của job tới hạn chờ lâu nhất, job trong
//! dead-letter queue, event chưa giao và tuổi của event cũ nhất, kết nối database đang dùng / còn
//! rảnh — theo database và module. Bộ đếm cộng dồn (`jobs_failed_total`…) không cho thấy hàng đợi
//! KẸT; các gauge này thì có.
//!
//! Hai phần tách nhau để test được trên cả hai database mà không đụng bộ thu số đo của tiến trình:
//! [`backlog`] chỉ đọc database và trả một [`BacklogSnapshot`]; [`publish`] chỉ đặt gauge.
//! [`BacklogMonitor`] chạy cả hai định kỳ ở tiến trình chạy job (vai `worker` / `all`).
//!
//! Nhãn: `database` (tên placement, bị chặn bởi `AUTOXIDE_METRICS_MAX_DATABASE_LABELS` — phần vượt
//! gộp vào `other`), `module` (module đã đăng ký: tập nhỏ cố định), `pool`, `state`. KHÔNG có nhãn
//! theo khách: tra theo khách đi qua log, trace và CLI (luật code về nhãn số đo).

use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Condition, Expr, ExprTrait as _, Func, Order, Query};

use crate::config::DatabaseBackend;

use crate::database::Database;
use crate::error::Error;
use crate::tenancy::DatabaseRegistry;

/// Nhãn `database` cho phần database vượt giới hạn.
pub const OTHER_DATABASE: &str = "other";

/// Tên mọi gauge [`publish`] đặt. Test của dashboard và luật cảnh báo trong `deploy/` so tên số đo
/// với danh sách này (cùng các số đo khác của kernel).
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

/// Tồn đọng của một database.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct BacklogSnapshot {
    /// Tên database (placement): `main`, hay tên placement của bước 33.
    pub database: String,
    pub modules: Vec<ModuleBacklog>,
    pub pools: Vec<PoolUsage>,
}

/// Tồn đọng của một module trên một database. Module không có bảng job (hay bảng event) thì các
/// số tương ứng là 0.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModuleBacklog {
    pub module: &'static str,
    /// Job chưa xong, chưa vào dead-letter queue (kể cả job hẹn giờ chưa tới).
    pub jobs_pending: u64,
    /// Tuổi (giây) của job TỚI HẠN chờ lâu nhất: `now - run_at`; 0 khi không có. Job hẹn giờ
    /// tương lai không tính — nó chưa chờ ai.
    pub jobs_oldest_pending_seconds: u64,
    /// Job trong dead-letter queue (`failed_at` có giá trị).
    pub jobs_dead: u64,
    /// Event chưa giao, chưa hỏng hẳn.
    pub events_pending: u64,
    /// Tuổi (giây) của event chưa giao cũ nhất: `now - occurred_at`; 0 khi không có.
    pub events_oldest_pending_seconds: u64,
    /// Event hỏng hẳn (`failed_at` có giá trị).
    pub events_dead: u64,
}

/// Kết nối của một pool.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PoolUsage {
    /// `read` hay `write`.
    pub pool: &'static str,
    pub in_use: u32,
    pub idle: u32,
    pub max: u32,
}

/// Đọc tồn đọng của `database` (tên `name`): bảng job của `job_modules`, bảng event của
/// `event_modules`, và hai pool. Chỉ dùng kết nối ĐỌC (SQLite chỉ có một kết nối ghi).
pub async fn backlog(
    database: &Database,
    name: &str,
    job_modules: &[&'static str],
    event_modules: &[&'static str],
    now: DateTime<Utc>,
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

/// Số dòng của `table` thoả `condition`.
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

/// Tuổi (giây, tính tới `now`) của dòng có `column` nhỏ nhất trong các dòng thoả `condition`; 0
/// khi không có dòng nào. Lấy dòng đầu theo thứ tự thay vì `MIN(...)`: kiểu trả về của hàm gộp
/// trên cột thời gian khác nhau giữa SQLite (chữ) và Postgres, còn đọc một cột thì như mọi chỗ khác.
async fn oldest(
    database: &Database,
    table: &str,
    column: &str,
    condition: Condition,
    now: DateTime<Utc>,
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
    let at: DateTime<Utc> = row.try_get("", column)?;
    Ok(u64::try_from((now - at).num_seconds()).unwrap_or(0))
}

/// Kết nối của hai pool. Đọc thẳng từ pool của sqlx (không truy vấn database).
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

/// Đặt gauge theo `snapshots`. Quá `max_database_labels` database thì phần còn lại (theo thứ tự
/// của `snapshots`) gộp vào nhãn `database="other"`: cộng số đếm, lấy tuổi lớn nhất — để số chuỗi
/// số đo có trần dù mỗi khách có database riêng.
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

/// Cập nhật gauge tồn đọng định kỳ cho mọi database máy chủ này chạy job. Dựng bằng
/// `Modules::backlog_monitor`; chạy ở tiến trình có vai `worker` hay `all`.
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

    /// Chu kỳ cập nhật (`AUTOXIDE_METRICS_BACKLOG_SECONDS`).
    pub fn every(mut self, every: Duration) -> Self {
        self.every = every.max(Duration::from_secs(1));
        self
    }

    /// Số nhãn `database` tối đa (`AUTOXIDE_METRICS_MAX_DATABASE_LABELS`).
    pub fn max_database_labels(mut self, max: usize) -> Self {
        self.max_database_labels = max.max(1);
        self
    }

    /// Một lượt: đọc tồn đọng của mọi database. Database lỗi thì ghi log rồi bỏ qua, không làm
    /// hỏng lượt của database khác.
    pub async fn collect(&self) -> Result<Vec<BacklogSnapshot>, Error> {
        let now = Utc::now();
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

    /// Chạy tới khi `shutdown` hoàn thành: mỗi chu kỳ một lượt [`collect`](Self::collect) rồi
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

    /// Quá giới hạn nhãn thì database còn lại gộp vào `other`: cộng số đếm, lấy tuổi lớn nhất.
    /// Dùng bộ thu số đo cục bộ để không đụng bộ thu của cả tiến trình.
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
