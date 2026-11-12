//! Worker: claim job từ bảng `<module>_jobs` của các module, chạy, ghi kết quả; đưa
//! job của lịch định kỳ vào hàng đợi; dọn job đã xong lâu.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use tokio::task::JoinSet;
use uuid::Uuid;

use super::queue::{self, now};
use super::{
    Claim, ClaimedJob, EnqueueOptions, JobContext, JobHandler, JobQueue, JobRecord, JobStatus,
    Schedule, claim_for, module_of, table_name,
};
use crate::database::Database;
use crate::error::Error;

/// Số lần chạy tối đa mặc định của một job.
pub const DEFAULT_MAX_ATTEMPTS: i32 = 10;
/// Một worker giữ một job tối đa chừng này; job chạy lâu hơn bị coi là treo.
const LEASE: Duration = Duration::from_secs(300);
/// Không có job thì hỏi lại sau chừng này.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// Giữ job đã xong chừng này rồi xoá.
const KEEP_COMPLETED: chrono::Duration = chrono::Duration::days(7);
/// Dọn job đã xong mỗi chừng này.
const CLEANUP_EVERY: Duration = Duration::from_secs(3600);
/// Độ dài tối đa của lỗi lưu lại.
const MAX_ERROR_LENGTH: usize = 2000;

/// Worker của chương trình.
pub struct Worker {
    database: Database,
    /// Các module có bảng job.
    modules: Vec<&'static str>,
    handlers: Arc<HashMap<&'static str, Arc<dyn JobHandler>>>,
    schedules: Vec<Schedule>,
    claim: Arc<dyn Claim>,
    /// Tên worker ghi vào `claimed_by`: phân biệt các tiến trình.
    id: String,
    /// Số job chạy cùng lúc.
    concurrency: u64,
    /// Khoảng thời gian gần nhất đã đưa job vào hàng đợi, theo tên lịch: khỏi thử chèn
    /// mỗi giây.
    last_slots: Mutex<HashMap<&'static str, i64>>,
    last_cleanup: Mutex<Option<Instant>>,
}

impl Worker {
    pub(crate) fn new(
        database: Database,
        modules: Vec<&'static str>,
        handlers: HashMap<&'static str, Arc<dyn JobHandler>>,
        schedules: Vec<Schedule>,
        concurrency: u64,
    ) -> Self {
        let claim: Arc<dyn Claim> = Arc::from(claim_for(database.write().get_database_backend()));
        Worker {
            database,
            modules,
            handlers: Arc::new(handlers),
            schedules,
            claim,
            id: format!("worker-{}", Uuid::now_v7()),
            concurrency: std::cmp::max(concurrency, 1),
            last_slots: Mutex::new(HashMap::new()),
            last_cleanup: Mutex::new(None),
        }
    }

    /// Một lượt: đưa job của lịch tới hạn vào hàng đợi, rồi chạy tối đa `concurrency`
    /// job của mỗi module. Trả về số job đã chạy.
    pub async fn run_once(&self) -> Result<usize, Error> {
        self.enqueue_schedules().await?;
        self.cleanup().await?;
        let mut ran = 0;
        for module in &self.modules {
            let now = now();
            let until = now + chrono::Duration::from_std(LEASE).map_err(Error::internal)?;
            let table = table_name(module);
            let claimed = self
                .claim
                .claim(self.database.write(), &table, &self.id, now, until, self.concurrency)
                .await?;
            ran += claimed.len();
            // Chạy song song; mỗi job một task. `JoinSet` đợi tất cả xong.
            let mut running = JoinSet::new();
            for job in claimed {
                let database = self.database.clone();
                let handlers = Arc::clone(&self.handlers);
                let (table, worker) = (table.clone(), self.id.clone());
                running.spawn(
                    async move { run_job(&database, &handlers, &table, &worker, job).await },
                );
            }
            while let Some(result) = running.join_next().await {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => tracing::error!(error = %error, "cannot record a job result"),
                    Err(error) => tracing::error!(error = %error, "a job task panicked"),
                }
            }
        }
        Ok(ran)
    }

    /// Chạy tới khi `shutdown` hoàn thành. Lỗi database thì ghi log rồi thử lại.
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        loop {
            let ran = match self.run_once().await {
                Ok(ran) => ran,
                Err(error) => {
                    tracing::error!(error = %error, "job worker failed; retrying");
                    0
                }
            };
            if ran > 0 {
                continue;
            }
            tokio::select! {
                () = &mut shutdown => return,
                () = tokio::time::sleep(IDLE_POLL) => {}
            }
        }
    }

    /// Job trạng thái `status` của module `module` (cho lệnh `autoxide jobs`).
    pub async fn list(
        &self,
        module: &str,
        status: JobStatus,
        limit: u64,
    ) -> Result<Vec<JobRecord>, Error> {
        self.known(module)?;
        queue::list(self.database.read(), module, status, limit).await
    }

    /// Đưa job hỏng hẳn `id` của module `module` trở lại hàng đợi.
    pub async fn retry(&self, module: &str, id: Uuid) -> Result<(), Error> {
        self.known(module)?;
        if queue::retry(self.database.write(), module, id).await? {
            tracing::info!(module, job_id = %id, "failed job queued again");
            return Ok(());
        }
        Err(Error::not_found(
            "job_not_found",
            "Module `{module}` has no failed job with id `{id}`. List failed jobs with \
             `autoxide jobs failed`.",
        )
        .with_param("module", module)
        .with_param("id", id))
    }

    /// Các module có bảng job.
    pub fn modules(&self) -> &[&'static str] {
        &self.modules
    }

    fn known(&self, module: &str) -> Result<(), Error> {
        if self.modules.contains(&module) {
            return Ok(());
        }
        Err(Error::not_found(
            "job_module_not_found",
            "Module `{module}` has no job queue. Modules with one: {modules}.",
        )
        .with_param("module", module)
        .with_param("modules", self.modules.join(", ")))
    }

    /// Mỗi lịch, mỗi khoảng thời gian: một job, chống trùng bằng `unique_key` — nhiều
    /// worker cùng chạy vẫn chỉ có một job cho mỗi khoảng.
    async fn enqueue_schedules(&self) -> Result<(), Error> {
        let now = now();
        for schedule in &self.schedules {
            let every =
                i64::try_from(std::cmp::max(schedule.every.as_secs(), 1)).unwrap_or(i64::MAX);
            let slot = now.timestamp().div_euclid(every);
            {
                let last = self.last_slots.lock().map_err(|_| Error::internal("schedule lock"))?;
                if last.get(schedule.name) == Some(&slot) {
                    continue;
                }
            }
            let module = module_of(schedule.kind).ok_or_else(|| {
                Error::internal(format!("schedule `{}` has an invalid job kind", schedule.name))
            })?;
            let module: &'static str =
                self.modules.iter().find(|known| **known == module).copied().ok_or_else(|| {
                    Error::internal(format!("module `{module}` has no job queue"))
                })?;
            let options = EnqueueOptions {
                run_at: DateTime::from_timestamp(slot * every, 0),
                max_attempts: None,
                unique_key: Some(format!("schedule:{}:{slot}", schedule.name)),
            };
            JobQueue::new(module)
                .enqueue_value(
                    self.database.write(),
                    schedule.kind,
                    schedule.payload.clone(),
                    options,
                )
                .await?;
            self.last_slots
                .lock()
                .map_err(|_| Error::internal("schedule lock"))?
                .insert(schedule.name, slot);
        }
        Ok(())
    }

    /// Xoá job đã xong hơn [`KEEP_COMPLETED`], mỗi giờ một lần.
    async fn cleanup(&self) -> Result<(), Error> {
        {
            let mut last = self.last_cleanup.lock().map_err(|_| Error::internal("cleanup lock"))?;
            if last.is_some_and(|at| at.elapsed() < CLEANUP_EVERY) {
                return Ok(());
            }
            *last = Some(Instant::now());
        }
        let before = now() - KEEP_COMPLETED;
        for module in &self.modules {
            let mut delete = Query::delete();
            delete
                .from_table(Alias::new(table_name(module)))
                .and_where(Expr::col(Alias::new("completed_at")).lt(before));
            self.database.write().execute(&delete).await?;
        }
        Ok(())
    }
}

/// Chạy một job đã claim và ghi kết quả.
async fn run_job(
    database: &Database,
    handlers: &HashMap<&'static str, Arc<dyn JobHandler>>,
    table: &str,
    worker: &str,
    job: ClaimedJob,
) -> Result<(), Error> {
    let outcome = match handlers.get(job.kind.as_str()) {
        None => Err(format!("no handler is registered for job kind `{}`", job.kind)),
        Some(handler) => match serde_json::from_str(&job.payload) {
            Err(error) => Err(format!("the payload is not valid JSON: {error}")),
            Ok(payload) => {
                let context = JobContext {
                    id: job.id,
                    kind: job.kind.clone(),
                    payload,
                    attempt: job.attempts,
                };
                // Dừng trước khi hết hạn giữ, để không worker nào khác chạy chồng.
                let limit = LEASE - Duration::from_secs(30);
                match tokio::time::timeout(limit, handler.run(&context)).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(error)) => Err(describe(&error)),
                    Err(_) => Err(format!("timed out after {} seconds", limit.as_secs())),
                }
            }
        },
    };
    let now = now();
    let mut update = Query::update();
    update
        .table(Alias::new(table))
        .value(Alias::new("claimed_by"), Option::<String>::None)
        .value(Alias::new("claimed_until"), Option::<DateTime<Utc>>::None)
        .and_where(Expr::col(Alias::new("id")).eq(job.id))
        // Chỉ ghi khi job còn là của worker này (chưa hết hạn giữ và bị worker khác lấy).
        .and_where(Expr::col(Alias::new("claimed_by")).eq(worker));
    match &outcome {
        Ok(()) => {
            update.value(Alias::new("completed_at"), now);
            // Xong rồi thì không cần dữ liệu nữa: xoá đi, để link chứa token (đặt lại mật
            // khẩu, xác minh email…) không nằm lại trong bảng job thêm nhiều ngày (D-079).
            update.value(Alias::new("payload"), "null");
            metrics::counter!("jobs_completed_total").increment(1);
        }
        Err(error) => {
            let message: String = error.chars().take(MAX_ERROR_LENGTH).collect();
            update.value(Alias::new("last_error"), message.clone());
            if job.attempts >= job.max_attempts {
                update.value(Alias::new("failed_at"), now);
                tracing::error!(
                    job_id = %job.id,
                    kind = %job.kind,
                    attempts = job.attempts,
                    error = %message,
                    "job failed for good; it is in the dead-letter queue"
                );
                metrics::counter!("jobs_failed_total").increment(1);
            } else {
                update.value(Alias::new("run_at"), now + backoff(job.attempts));
                tracing::warn!(
                    job_id = %job.id,
                    kind = %job.kind,
                    attempts = job.attempts,
                    error = %message,
                    "job failed; will retry"
                );
                metrics::counter!("jobs_retried_total").increment(1);
            }
        }
    }
    database.write().execute(&update).await?;
    Ok(())
}

/// Chờ trước lần chạy thứ `attempts + 1`: 2, 4, 8… giây, tối đa một giờ.
fn backoff(attempts: i32) -> chrono::Duration {
    let exponent = u32::try_from(attempts.clamp(1, 12)).unwrap_or(12);
    chrono::Duration::seconds(std::cmp::min(i64::from(2u32.pow(exponent)), 3600))
}

/// Lỗi kèm nguyên nhân gốc, để người vận hành biết sửa gì.
fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => format!("{}: {}", error.code(), error.message()),
    }
}
