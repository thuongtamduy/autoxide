//! Worker: claim job từ bảng `<module>_jobs` của các module, chạy, ghi kết quả; đưa
//! job của lịch định kỳ vào hàng đợi; dọn job đã xong lâu.
//!
//! Nhiều database (bước 33): mỗi lượt duyệt mọi database được giao (`DatabaseRegistry::databases`,
//! lọc theo `AUTOXIDE_WORKER_PLACEMENTS`). Lịch chạy mỗi database một lần, với
//! [`NO_TENANT`]. **Chia lượt công bằng**: trong một database × module, các khách đang có job tới
//! hạn chia đều số chỗ trống (ít nhất một mỗi khách, điểm bắt đầu xoay vòng giữa các lượt); chỗ
//! còn thừa đi cho job tới hạn sớm nhất của bất kỳ khách nào.

use crate::time::Timestamp;
use futures_util::FutureExt as _;
use jiff::SignedDuration;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
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
use crate::tenancy::{DatabaseRegistry, NO_TENANT, PlacementId, TenantId, TenantStatus};

/// Số lần chạy tối đa mặc định của một job.
pub const DEFAULT_MAX_ATTEMPTS: i32 = 10;
/// Một worker giữ một job tối đa chừng này; job chạy lâu hơn bị coi là treo.
const LEASE: Duration = Duration::from_secs(300);
/// Không có job thì hỏi lại sau chừng này.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// Giữ job đã xong chừng này rồi xoá.
const KEEP_COMPLETED: SignedDuration = SignedDuration::from_hours(24 * 7);
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
    /// Thời gian chạy tối đa riêng của loại job (`JobRegistration::timeout`).
    timeouts: Arc<HashMap<&'static str, Duration>>,
    /// Hạn giữ một job đã claim ([`LEASE`]; test đặt ngắn hơn).
    lease: Duration,
    schedules: Vec<Schedule>,
    /// Tên worker ghi vào `claimed_by`: phân biệt các tiến trình.
    id: String,
    /// Số job chạy cùng lúc.
    concurrency: u64,
    /// Khoảng thời gian gần nhất đã đưa job vào hàng đợi, theo (database, tên lịch): khỏi thử
    /// chèn mỗi giây.
    last_slots: Mutex<HashMap<(PlacementId, &'static str), i64>>,
    last_cleanup: Mutex<Option<Instant>>,
    /// Luồng thay đổi đồng bộ của các module (bước 69 P2): dọn dòng quá hạn cùng lượt dọn job.
    sync: Vec<crate::sync::SyncLog>,
    /// Điểm bắt đầu xoay vòng giữa các khách; tăng mỗi lượt chia chỗ.
    turn: AtomicUsize,
    /// Quyền dùng theo khách (bước 35): job của module mà gói của khách không có bị hoãn.
    gate: Gate,
}

/// Điều kiện chạy job theo gói của khách (bước 35, D-325).
#[derive(Clone, Default)]
pub(crate) struct Gate {
    pub entitlements: crate::entitlement::EntitlementsHandle,
    /// Hoãn job bị chặn chừng này (không tính lượt thử).
    pub delay: Option<SignedDuration>,
}

impl Gate {
    /// Job của module `module` cho khách `tenant` có bị gói chặn không: module chưa bật cho khách,
    /// hay khách chỉ đọc vì thuê bao. Module `always_enabled` không bao giờ bị chặn.
    async fn blocks(
        &self,
        module: &str,
        tenant: &crate::tenancy::TenantContext,
    ) -> Result<bool, Error> {
        if !self.entitlements.is_enforced() || self.entitlements.catalog().is_always_enabled(module)
        {
            return Ok(false);
        }
        let snapshot = self.entitlements.snapshot(tenant.id()).await?;
        Ok(!snapshot.has_module(module)
            || snapshot.access == crate::entitlement::TenantAccess::ReadOnly)
    }
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
            timeouts: Arc::new(HashMap::new()),
            lease: LEASE,
            schedules,
            id: format!("worker-{}", Uuid::now_v7()),
            concurrency: std::cmp::max(concurrency, 1),
            last_slots: Mutex::new(HashMap::new()),
            last_cleanup: Mutex::new(None),
            sync: Vec::new(),
            turn: AtomicUsize::new(0),
            gate: Gate::default(),
        }
    }

    /// Luồng thay đổi đồng bộ cần dọn (bước 69 P2).
    pub(crate) fn with_sync(mut self, sync: Vec<crate::sync::SyncLog>) -> Self {
        self.sync = sync;
        self
    }

    pub(crate) fn with_timeouts(mut self, timeouts: HashMap<&'static str, Duration>) -> Self {
        self.timeouts = Arc::new(timeouts);
        self
    }

    /// Hạn giữ một job đã claim (mặc định 5 phút). Chỉ để test việc gia hạn với hạn ngắn; chương
    /// trình không đổi nó.
    #[doc(hidden)]
    pub fn lease(mut self, lease: Duration) -> Self {
        self.lease = lease.max(Duration::from_secs(1));
        self
    }

    /// Hoãn job của module mà gói của khách không có (bước 35). `Modules::worker` gọi.
    pub(crate) fn with_entitlements(
        mut self,
        entitlements: crate::entitlement::EntitlementsHandle,
    ) -> Self {
        self.gate.entitlements = entitlements;
        self
    }

    /// Hoãn job bị gói chặn `delay` (mặc định một giờ, `AUTOXIDE_JOBS_BLOCKED_DELAY_SECONDS`).
    pub fn blocked_delay(mut self, delay: Duration) -> Self {
        self.gate.delay = SignedDuration::try_from(delay).ok();
        self
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
        for (placement, _, database) in &databases {
            for module in &self.modules {
                for job in self.claim(database, module, self.concurrency).await? {
                    ran += 1;
                    running.spawn(self.job_task(*placement, database, module, job, None));
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
        for (placement, _, database) in &databases {
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
                    running.spawn(self.job_task(*placement, database, module, job, Some(permit)));
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
        let until = now + SignedDuration::try_from(self.lease).map_err(Error::internal)?;
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

    /// Task chạy `job` của `module` trên `database` (placement `placement`); giữ `permit` (chỗ của
    /// module) tới khi xong.
    fn job_task(
        &self,
        placement: PlacementId,
        database: &Database,
        module: &str,
        job: ClaimedJob,
        permit: Option<OwnedSemaphorePermit>,
    ) -> impl Future<Output = ()> + Send + 'static {
        let database = database.clone();
        let registry = self.databases.clone();
        let handlers = Arc::clone(&self.handlers);
        let timeouts = Arc::clone(&self.timeouts);
        let gate = self.gate.clone();
        let lease = self.lease;
        let (table, worker) = (table_name(module), self.id.clone());
        async move {
            let env = JobEnv {
                registry: &registry,
                handlers: &handlers,
                timeouts: &timeouts,
                gate: &gate,
                lease,
            };
            let run = run_job(placement, &database, &env, &table, &worker, job);
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
                if schedule.control_only && *placement != crate::tenancy::MAIN_PLACEMENT {
                    continue;
                }
                let every =
                    i64::try_from(std::cmp::max(schedule.every.as_secs(), 1)).unwrap_or(i64::MAX);
                let slot = now.as_unix_seconds().div_euclid(every);
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
                    run_at: Timestamp::from_unix_seconds(slot * every),
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
            for log in &self.sync {
                log.purge(database).await?;
            }
        }
        Ok(())
    }
}

/// Khách đang có job tới hạn trong bảng `table` (tối đa [`MAX_TENANTS_PER_ROUND`]).
async fn due_tenants(
    database: &Database,
    table: &str,
    now: Timestamp,
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
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
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
///
/// Khách đang dời sang placement khác (`moving`): trả job về hàng đợi, không tính lần chạy — dữ liệu
/// của khách đang được chép, job chạy ở đích sau khi dời xong. Khách đã ở placement khác: dòng job
/// này là bản sao còn sót ở nguồn (dời với `keep_source`) — không chạy, đánh dấu hỏng hẳn.
/// Thứ mọi job cần để chạy: sổ database (tìm khách), bộ chạy theo loại, điều kiện theo gói.
struct JobEnv<'a> {
    registry: &'a DatabaseRegistry,
    handlers: &'a HashMap<&'static str, Arc<dyn JobHandler>>,
    timeouts: &'a HashMap<&'static str, Duration>,
    gate: &'a Gate,
    lease: Duration,
}

async fn run_job(
    placement: PlacementId,
    database: &Database,
    env: &JobEnv<'_>,
    table: &str,
    worker: &str,
    job: ClaimedJob,
) -> Result<(), Error> {
    let JobEnv { registry, handlers, timeouts, gate, lease } = env;
    // Khách của job (`NO_TENANT`: job của cả database). Khách không còn: một lần lỗi như mọi lỗi.
    let tenant = if job.tenant == NO_TENANT {
        Ok(None)
    } else {
        registry.tenant(job.tenant).await.map(Some).map_err(|error| describe(&error))
    };
    if let Ok(Some(tenant)) = &tenant {
        if tenant.status() == TenantStatus::Moving {
            return put_back(database, table, worker, &job).await;
        }
        if tenant.placement() != placement {
            return left_over(database, table, worker, &job, tenant.placement()).await;
        }
        let module = crate::job::module_of(&job.kind).unwrap_or_default();
        if gate.blocks(module, tenant).await? {
            metrics::counter!("entitlement_denials_total", "kind" => "job").increment(1);
            let delay = gate.delay.unwrap_or(BLOCKED_DELAY);
            return put_back_after(database, table, worker, &job, delay).await;
        }
    }
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
                // Mặc định dừng trước khi hết hạn giữ, để không worker nào khác chạy chồng. Loại
                // job khai thời gian dài hơn (`JobRegistration::timeout`) thì gia hạn giữ trong lúc
                // chạy (bước 39).
                let short =
                    std::cmp::max(lease.saturating_sub(Duration::from_secs(30)), *lease / 2);
                let limit = timeouts.get(job.kind.as_str()).copied().unwrap_or(short);
                // Handler panic: ghi như một lần lỗi (thử lại, rồi vào dead-letter queue) thay
                // vì để job nằm đó tới hết hạn giữ rồi chạy lại mãi (D-158).
                // Span của job mang khách (bước 39 B7, D-190): mọi log trong lúc chạy tra được theo khách.
                let span = tracing::info_span!(
                    "job",
                    job_id = %job.id,
                    kind = %job.kind,
                    tenant_id = tracing::field::Empty,
                );
                if let Some(tenant) = &context.tenant {
                    span.record("tenant_id", tracing::field::display(tenant.id()));
                }
                let run =
                    AssertUnwindSafe(tracing::Instrument::instrument(handler.run(&context), span))
                        .catch_unwind();
                let timed = tokio::time::timeout(limit, run);
                let outcome = if limit > short {
                    // `select!` chạy hai việc cùng lúc và lấy việc xong trước: job xong thì bỏ
                    // vòng gia hạn; mất quyền giữ thì bỏ (drop) future của job — job bị dừng.
                    tokio::select! {
                        outcome = timed => Some(outcome),
                        lost = keep_claim(database, table, worker, job.id, *lease) => {
                            tracing::warn!(
                                job_id = %job.id,
                                kind = %job.kind,
                                error = %lost,
                                "job stopped: this worker lost its claim on it"
                            );
                            None
                        }
                    }
                } else {
                    Some(timed.await)
                };
                match outcome {
                    // Không còn là job của worker này: không ghi kết quả (lệnh ghi bên dưới chỉ ghi
                    // khi `claimed_by` còn là worker này nên cũng không ghi được).
                    None => return Ok(()),
                    Some(Ok(Ok(Ok(())))) => Ok(()),
                    Some(Ok(Ok(Err(error)))) => Err(describe(&error)),
                    Some(Ok(Err(_))) => Err("the job handler panicked".to_owned()),
                    Some(Err(_)) => Err(format!("timed out after {} seconds", limit.as_secs())),
                }
            }
        },
    };
    let now = now();
    let mut update = Query::update();
    update
        .table(Alias::new(table))
        .value(Alias::new("claimed_by"), Option::<String>::None)
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
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
    // Job người dùng theo dõi (bước 51 P15): xong hay hỏng hẳn thì ghi lên dòng theo dõi, nếu có.
    let finished = match &outcome {
        Ok(()) => {
            crate::job::runs::finish(database.write(), job.id, None).await?;
            Some(false)
        }
        Err(error) if job.attempts >= job.max_attempts => {
            crate::job::runs::finish(database.write(), job.id, Some(error)).await?;
            Some(true)
        }
        Err(_) => None,
    };
    // Báo người yêu cầu (bước 51): lỗi khi phát event không làm job chạy lại — job đã xong.
    if let Some(failed) = finished
        && job.tenant != NO_TENANT
        && let Ok(tenant) = registry.tenant(job.tenant).await
        && let Err(error) = crate::job::runs::announce(&tenant, job.id, failed).await
    {
        tracing::warn!(job_id = %job.id, error = %error, "job_finished event not recorded");
    }
    Ok(())
}

/// Gia hạn giữ job `id` mỗi một phần ba `lease`, tới khi mất quyền giữ (không còn dòng nào
/// `claimed_by` là worker này) hay lỗi database liên tiếp — khi đó trả lý do. Chỉ trả về khi phải
/// dừng job; job xong trước thì future này bị bỏ.
async fn keep_claim(
    database: &Database,
    table: &str,
    worker: &str,
    id: Uuid,
    lease: Duration,
) -> String {
    let every = std::cmp::max(lease / 3, Duration::from_millis(200));
    let Ok(length) = SignedDuration::try_from(lease) else {
        return "the lease is too long".to_owned();
    };
    loop {
        tokio::time::sleep(every).await;
        let mut update = Query::update();
        update
            .table(Alias::new(table))
            .value(Alias::new("claimed_until"), now() + length)
            .and_where(Expr::col(Alias::new("id")).eq(id))
            .and_where(Expr::col(Alias::new("claimed_by")).eq(worker))
            .and_where(Expr::col(Alias::new("completed_at")).is_null())
            .and_where(Expr::col(Alias::new("failed_at")).is_null());
        match database.write().execute(&update).await {
            Ok(result) if result.rows_affected() == 0 => {
                return "another worker or a command took the job back".to_owned();
            }
            Ok(_) => {}
            // Một lần lỗi database: thử lại lần sau; hạn giữ còn hai phần ba.
            Err(error) => {
                tracing::warn!(job_id = %id, error = %error, "cannot extend a job's lease")
            }
        }
    }
}

/// Khách đang dời: chờ sau chừng này rồi mới nhận lại job của khách.
const MOVING_DELAY: SignedDuration = SignedDuration::from_secs(30);

/// Job bị gói chặn: chờ sau chừng này (khi chương trình không đặt khác).
const BLOCKED_DELAY: SignedDuration = SignedDuration::from_secs(3600);

/// Trả job vừa claim về hàng đợi (khách đang dời), không tính lần chạy này.
async fn put_back(
    database: &Database,
    table: &str,
    worker: &str,
    job: &ClaimedJob,
) -> Result<(), Error> {
    put_back_after(database, table, worker, job, MOVING_DELAY).await
}

/// Trả job vừa claim về hàng đợi, chạy lại sau `delay`, không tính lần chạy này.
async fn put_back_after(
    database: &Database,
    table: &str,
    worker: &str,
    job: &ClaimedJob,
    delay: SignedDuration,
) -> Result<(), Error> {
    let mut update = Query::update();
    update
        .table(Alias::new(table))
        .value(Alias::new("claimed_by"), Option::<String>::None)
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .value(Alias::new("attempts"), Expr::col(Alias::new("attempts")).sub(1))
        .value(Alias::new("run_at"), now() + delay)
        .and_where(Expr::col(Alias::new("id")).eq(job.id))
        .and_where(Expr::col(Alias::new("claimed_by")).eq(worker));
    database.write().execute(&update).await?;
    tracing::debug!(job_id = %job.id, tenant_id = %job.tenant, "job put back for later");
    Ok(())
}

/// Job của khách đã dời sang placement khác: bản sao còn sót ở nguồn, không chạy.
async fn left_over(
    database: &Database,
    table: &str,
    worker: &str,
    job: &ClaimedJob,
    now_at: PlacementId,
) -> Result<(), Error> {
    let message = format!(
        "the tenant's data moved to placement {now_at}; this job is a left-over copy at the old \
         placement and is not run"
    );
    let mut update = Query::update();
    update
        .table(Alias::new(table))
        .value(Alias::new("claimed_by"), Option::<String>::None)
        .value(Alias::new("claimed_until"), Option::<Timestamp>::None)
        .value(Alias::new("failed_at"), now())
        .value(Alias::new("last_error"), message)
        .and_where(Expr::col(Alias::new("id")).eq(job.id))
        .and_where(Expr::col(Alias::new("claimed_by")).eq(worker));
    database.write().execute(&update).await?;
    tracing::warn!(job_id = %job.id, tenant_id = %job.tenant, "left-over job of a moved tenant not run");
    Ok(())
}

/// Chờ trước lần chạy thứ `attempts + 1`: 2, 4, 8… giây, tối đa một giờ.
fn backoff(attempts: i32) -> SignedDuration {
    let exponent = u32::try_from(attempts.clamp(1, 12)).unwrap_or(12);
    SignedDuration::from_secs(std::cmp::min(i64::from(2u32.pow(exponent)), 3600))
}

/// Lỗi kèm nguyên nhân gốc, để người vận hành biết sửa gì.
fn describe(error: &Error) -> String {
    match std::error::Error::source(error) {
        Some(source) => format!("{}: {source}", error.code()),
        None => format!("{}: {}", error.code(), error.message()),
    }
}
