//! Worker: claim job từ bảng `<module>_jobs` của các module, chạy, ghi kết quả; đưa
//! job của lịch định kỳ vào hàng đợi; dọn job đã xong lâu.
//!
//! Nhiều database (bước 33): mỗi lượt duyệt mọi database được giao (`DatabaseRegistry::databases`,
//! lọc theo `AUTOXIDE_WORKER_PLACEMENTS`). Lịch chạy mỗi database một lần, với
//! [`NO_TENANT`]. **Chia lượt công bằng**: trong một database × module, các khách đang có job tới
//! hạn chia đều số chỗ trống (ít nhất một mỗi khách, điểm bắt đầu xoay vòng giữa các lượt); chỗ
//! còn thừa đi cho job tới hạn sớm nhất của bất kỳ khách nào.

use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use futures_util::FutureExt as _;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use uuid::Uuid;

use super::claim::pending;
use super::queue::{self, now};
use super::{
    ClaimRequest, ClaimedJob, EnqueueOptions, JobContext, JobHandler, JobQueue, JobRecord,
    JobStatus, Schedule, claim_for, module_of, table_name,
};
use crate::database::Database;
use crate::error::Error;
use crate::tenancy::{DatabaseRegistry, NO_TENANT, PlacementId, TenantId};

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
/// Số khách tối đa xét trong một lượt chia chỗ của một database × module; khách sau lượt này chờ
/// lượt sau (điểm bắt đầu xoay vòng).
const MAX_TENANTS_PER_ROUND: u64 = 256;

/// Một database được giao cho worker: placement và kết nối.
type Placement = (PlacementId, String, Database);

/// Worker của chương trình.
pub struct Worker {
    databases: DatabaseRegistry,
    /// Các module có bảng job.
    modules: Vec<&'static str>,
    handlers: Arc<HashMap<&'static str, Arc<dyn JobHandler>>>,
    schedules: Vec<Schedule>,
    /// Tên worker ghi vào `claimed_by`: phân biệt các tiến trình.
    id: String,
    /// Số job chạy cùng lúc.
    concurrency: u64,
    /// Khoảng thời gian gần nhất đã đưa job vào hàng đợi, theo (database, tên lịch): khỏi thử
    /// chèn mỗi giây.
    last_slots: Mutex<HashMap<(PlacementId, &'static str), i64>>,
    last_cleanup: Mutex<Option<Instant>>,
    /// Điểm bắt đầu xoay vòng giữa các khách; tăng mỗi lượt chia chỗ.
    turn: AtomicUsize,
}

impl Worker {
    pub(crate) fn new(
        databases: DatabaseRegistry,
        modules: Vec<&'static str>,
        handlers: HashMap<&'static str, Arc<dyn JobHandler>>,
        schedules: Vec<Schedule>,
        concurrency: u64,
    ) -> Self {
        Worker {
            databases,
            modules,
            handlers: Arc::new(handlers),
            schedules,
            id: format!("worker-{}", Uuid::now_v7()),
            concurrency: std::cmp::max(concurrency, 1),
            last_slots: Mutex::new(HashMap::new()),
            last_cleanup: Mutex::new(None),
            turn: AtomicUsize::new(0),
        }
    }

    /// Một lượt: đưa job của lịch tới hạn vào hàng đợi, rồi chạy tối đa `concurrency`
    /// job của mỗi module trên mỗi database và chờ chúng xong. Trả về số job đã chạy. Cho test và
    /// công cụ; chương trình dùng [`run`](Self::run), không chờ theo lượt.
    pub async fn run_once(&self) -> Result<usize, Error> {
        let databases = self.databases.databases().await?;
        self.enqueue_schedules(&databases).await?;
        self.cleanup(&databases).await?;
        let mut running = JoinSet::new();
        let mut ran = 0;
        for (_, _, database) in &databases {
            for module in &self.modules {
                for job in self.claim(database, module, self.concurrency).await? {
                    ran += 1;
                    running.spawn(self.job_task(database, module, job, None));
                }
            }
        }
        while let Some(result) = running.join_next().await {
            log_task(result);
        }
        Ok(ran)
    }

    /// Chạy tới khi `shutdown` hoàn thành, rồi chờ các job đang chạy xong. Lỗi database thì
    /// ghi log rồi thử lại.
    ///
    /// Mỗi module tối đa `concurrency` job cùng lúc (cộng mọi database), độc lập với nhau: một job
    /// chậm (gọi AI, webhook tới máy chủ chậm) chỉ giữ chỗ của chính nó — job khác, của module
    /// khác (mail đặt lại mật khẩu) vẫn được nhận ngay khi có chỗ (D-158).
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        tokio::pin!(shutdown);
        let slots: HashMap<&'static str, Arc<Semaphore>> = self
            .modules
            .iter()
            .map(|module| {
                let permits = usize::try_from(self.concurrency).unwrap_or(usize::MAX);
                (*module, Arc::new(Semaphore::new(permits)))
            })
            .collect();
        let mut running = JoinSet::new();
        loop {
            let claimed = match self.claim_free(&slots, &mut running).await {
                Ok(claimed) => claimed,
                Err(error) => {
                    tracing::error!(error = %error, "job worker failed; retrying");
                    0
                }
            };
            if claimed > 0 {
                continue;
            }
            // Không có gì mới: chờ một job xong (có chỗ), hết giờ chờ, hay lệnh dừng.
            tokio::select! {
                () = &mut shutdown => break,
                Some(result) = running.join_next() => log_task(result),
                () = tokio::time::sleep(IDLE_POLL) => {}
            }
        }
        while let Some(result) = running.join_next().await {
            log_task(result);
        }
    }

    /// Một vòng của [`run`](Self::run): lịch, dọn dẹp, rồi claim cho mỗi module đúng số chỗ
    /// còn trống. Trả số job vừa nhận.
    async fn claim_free(
        &self,
        slots: &HashMap<&'static str, Arc<Semaphore>>,
        running: &mut JoinSet<()>,
    ) -> Result<usize, Error> {
        while let Some(result) = running.try_join_next() {
            log_task(result);
        }
        let databases = self.databases.databases().await?;
        self.enqueue_schedules(&databases).await?;
        self.cleanup(&databases).await?;
        let mut claimed = 0;
        for (_, _, database) in &databases {
            for module in &self.modules {
                let Some(slots) = slots.get(module) else { continue };
                let free = u64::try_from(slots.available_permits()).unwrap_or(0);
                if free == 0 {
                    continue;
                }
                for job in self.claim(database, module, free).await? {
                    // Còn chỗ (vừa đếm, và chỉ vòng này lấy chỗ): `try_acquire` không hụt.
                    let Ok(permit) = Arc::clone(slots).try_acquire_owned() else { break };
                    claimed += 1;
                    running.spawn(self.job_task(database, module, job, Some(permit)));
                }
            }
        }
        Ok(claimed)
    }

    /// Claim tối đa `limit` job tới hạn của `module` trên `database`, chia đều giữa các khách.
    async fn claim(
        &self,
        database: &Database,
        module: &str,
        limit: u64,
    ) -> Result<Vec<ClaimedJob>, Error> {
        let now = now();
        let until = now + chrono::Duration::from_std(LEASE).map_err(Error::internal)?;
        let table = table_name(module);
        let claim = claim_for(database.write().get_database_backend());
        let connection = database.write();
        let request = ClaimRequest::new(&table, &self.id, now, until, limit);
        let tenants = due_tenants(database, &table, now).await?;
        if tenants.len() <= 1 {
            // Không ai, hay chỉ một khách: không có gì để chia.
            return if tenants.is_empty() {
                Ok(Vec::new())
            } else {
                claim.claim(connection, &request).await
            };
        }
        let count = tenants.len();
        let share = std::cmp::max(limit / u64::try_from(count).unwrap_or(u64::MAX), 1);
        let start = self.turn.fetch_add(1, Ordering::Relaxed) % count;
        let mut claimed = Vec::new();
        for tenant in tenants.iter().cycle().skip(start).take(count) {
            let left = limit.saturating_sub(u64::try_from(claimed.len()).unwrap_or(u64::MAX));
            if left == 0 {
                break;
            }
            let mut request = request.clone().tenant(*tenant);
            request.limit = std::cmp::min(share, left);
            claimed.extend(claim.claim(connection, &request).await?);
        }
        // Chỗ còn thừa (khách có ít job hơn phần chia): job tới hạn sớm nhất của bất kỳ ai.
        let left = limit.saturating_sub(u64::try_from(claimed.len()).unwrap_or(u64::MAX));
        if left > 0 {
            let mut request = request;
            request.limit = left;
            claimed.extend(claim.claim(connection, &request).await?);
        }
        Ok(claimed)
    }

    /// Task chạy `job` của `module`; giữ `permit` (chỗ của module) tới khi xong.
    fn job_task(
        &self,
        database: &Database,
        module: &str,
        job: ClaimedJob,
        permit: Option<OwnedSemaphorePermit>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let database = database.clone();
        let registry = self.databases.clone();
        let handlers = Arc::clone(&self.handlers);
        let (table, worker) = (table_name(module), self.id.clone());
        async move {
            let run = run_job(&database, &registry, &handlers, &table, &worker, job);
            if let Err(error) = run.await {
                tracing::error!(error = %error, "cannot record a job result");
            }
            drop(permit);
        }
    }

    /// Job trạng thái `status` của module `module` trên mọi database (cho lệnh `autoxide jobs`),
    /// mới nhất trước.
    pub async fn list(
        &self,
        module: &str,
        status: JobStatus,
        limit: u64,
    ) -> Result<Vec<JobRecord>, Error> {
        self.known(module)?;
        let mut all = Vec::new();
        for (_, _, database) in self.databases.databases().await? {
            all.extend(queue::list(database.read(), module, status, limit).await?);
        }
        all.sort_by_key(|record| std::cmp::Reverse(record.id));
        all.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(all)
    }

    /// Đưa job hỏng hẳn `id` của module `module` trở lại hàng đợi.
    pub async fn retry(&self, module: &str, id: Uuid) -> Result<(), Error> {
        self.known(module)?;
        for (_, _, database) in self.databases.databases().await? {
            if queue::retry(database.write(), module, id).await? {
                tracing::info!(module, job_id = %id, "failed job queued again");
                return Ok(());
            }
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

    /// Mỗi lịch, mỗi database, mỗi khoảng thời gian: một job của cả database (`NO_TENANT`),
    /// chống trùng bằng `unique_key` — nhiều worker cùng chạy vẫn chỉ có một job cho mỗi khoảng.
    async fn enqueue_schedules(&self, databases: &[Placement]) -> Result<(), Error> {
        let now = now();
        for (placement, _, database) in databases {
            for schedule in &self.schedules {
                let every =
                    i64::try_from(std::cmp::max(schedule.every.as_secs(), 1)).unwrap_or(i64::MAX);
                let slot = now.timestamp().div_euclid(every);
                let key = (*placement, schedule.name);
                {
                    let last =
                        self.last_slots.lock().map_err(|_| Error::internal("schedule lock"))?;
                    if last.get(&key) == Some(&slot) {
                        continue;
                    }
                }
                let module = module_of(schedule.kind).ok_or_else(|| {
                    Error::internal(format!("schedule `{}` has an invalid job kind", schedule.name))
                })?;
                let module: &'static str =
                    self.modules.iter().find(|known| **known == module).copied().ok_or_else(
                        || Error::internal(format!("module `{module}` has no job queue")),
                    )?;
                let options = EnqueueOptions {
                    run_at: DateTime::from_timestamp(slot * every, 0),
                    max_attempts: None,
                    unique_key: Some(format!("schedule:{}:{slot}", schedule.name)),
                };
                JobQueue::new(module)
                    .enqueue_value(
                        database.write(),
                        NO_TENANT,
                        schedule.kind,
                        schedule.payload.clone(),
                        options,
                    )
                    .await?;
                self.last_slots
                    .lock()
                    .map_err(|_| Error::internal("schedule lock"))?
                    .insert(key, slot);
            }
        }
        Ok(())
    }

    /// Xoá job đã xong hơn [`KEEP_COMPLETED`], mỗi giờ một lần, trên mọi database.
    async fn cleanup(&self, databases: &[Placement]) -> Result<(), Error> {
        {
            let mut last = self.last_cleanup.lock().map_err(|_| Error::internal("cleanup lock"))?;
            if last.is_some_and(|at| at.elapsed() < CLEANUP_EVERY) {
                return Ok(());
            }
            *last = Some(Instant::now());
        }
        for (_, _, database) in databases {
            for module in &self.modules {
                cleanup_table(database, module).await?;
            }
        }
        Ok(())
    }
}

/// Khách đang có job tới hạn trong bảng `table` (tối đa [`MAX_TENANTS_PER_ROUND`]).
async fn due_tenants(
    database: &Database,
    table: &str,
    now: DateTime<Utc>,
) -> Result<Vec<TenantId>, Error> {
    let select = Query::select()
        .distinct()
        .column(Alias::new("tenant_id"))
        .from(Alias::new(table))
        .cond_where(pending(now))
        .limit(MAX_TENANTS_PER_ROUND)
        .to_owned();
    let rows = database.write().query_all(&select).await?;
    let mut tenants = rows
        .iter()
        .map(|row| Ok(TenantId::from_uuid(row.try_get("", "tenant_id")?)))
        .collect::<Result<Vec<_>, Error>>()?;
    // Thứ tự ổn định để xoay vòng có nghĩa.
    tenants.sort();
    Ok(tenants)
}

/// Dọn bảng job của `module` trên `database`: xoá job đã xong lâu; job chạy đủ lần mà chưa ghi
/// được kết quả vào dead-letter queue.
async fn cleanup_table(database: &Database, module: &str) -> Result<(), Error> {
    let now = now();
    let before = now - KEEP_COMPLETED;
    let mut delete = Query::delete();
    delete
        .from_table(Alias::new(table_name(module)))
        .and_where(Expr::col(Alias::new("completed_at")).lt(before));
    database.write().execute(&delete).await?;
    // Job đã chạy đủ số lần mà lần cuối không ghi được kết quả (tiến trình chết giữa
    // chừng) và đã hết hạn giữ: vào dead-letter queue (D-158).
    let mut exhausted = Query::update();
    exhausted
        .table(Alias::new(table_name(module)))
        .value(Alias::new("failed_at"), now)
        .value(
            Alias::new("last_error"),
            "the worker stopped during the last attempt (crash or restart)",
        )
        .value(Alias::new("claimed_by"), Option::<String>::None)
        .value(Alias::new("claimed_until"), Option::<DateTime<Utc>>::None)
        .and_where(Expr::col(Alias::new("completed_at")).is_null())
        .and_where(Expr::col(Alias::new("failed_at")).is_null())
        .and_where(Expr::col(Alias::new("attempts")).gte(Expr::col(Alias::new("max_attempts"))))
        .and_where(Expr::col(Alias::new("claimed_until")).lt(now));
    let moved = database.write().execute(&exhausted).await?.rows_affected();
    if moved > 0 {
        tracing::error!(
            module,
            jobs = moved,
            "jobs moved to the dead-letter queue after the worker stopped mid-run"
        );
    }
    Ok(())
}

/// Ghi log task job kết thúc bất thường (đáng lẽ không xảy ra: `run_job` tự bắt panic).
fn log_task(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::error!(error = %error, "a job task ended abnormally");
    }
}

/// Chạy một job đã claim và ghi kết quả.
async fn run_job(
    database: &Database,
    registry: &DatabaseRegistry,
    handlers: &HashMap<&'static str, Arc<dyn JobHandler>>,
    table: &str,
    worker: &str,
    job: ClaimedJob,
) -> Result<(), Error> {
    // Khách của job (`NO_TENANT`: job của cả database). Khách không còn: một lần lỗi như mọi lỗi.
    let tenant = if job.tenant == NO_TENANT {
        Ok(None)
    } else {
        registry.tenant(job.tenant).await.map(Some).map_err(|error| describe(&error))
    };
    let outcome = match (handlers.get(job.kind.as_str()), tenant) {
        (None, _) => Err(format!("no handler is registered for job kind `{}`", job.kind)),
        (_, Err(error)) => Err(format!("tenant `{}`: {error}", job.tenant)),
        (Some(handler), Ok(tenant)) => match serde_json::from_str(&job.payload) {
            Err(error) => Err(format!("the payload is not valid JSON: {error}")),
            Ok(payload) => {
                let context = JobContext {
                    id: job.id,
                    kind: job.kind.clone(),
                    payload,
                    attempt: job.attempts,
                    tenant,
                    database: database.clone(),
                };
                // Dừng trước khi hết hạn giữ, để không worker nào khác chạy chồng.
                let limit = LEASE - Duration::from_secs(30);
                // Handler panic: ghi như một lần lỗi (thử lại, rồi vào dead-letter queue) thay
                // vì để job nằm đó tới hết hạn giữ rồi chạy lại mãi (D-158).
                let run = AssertUnwindSafe(handler.run(&context)).catch_unwind();
                match tokio::time::timeout(limit, run).await {
                    Ok(Ok(Ok(()))) => Ok(()),
                    Ok(Ok(Err(error))) => Err(describe(&error)),
                    Ok(Err(_)) => Err("the job handler panicked".to_owned()),
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
