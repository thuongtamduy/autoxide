//! Job queue trên database gate đang chọn: SQLite dùng nhánh claim `UPDATE … RETURNING`,
//! Postgres dùng nhánh `FOR UPDATE SKIP LOCKED` — gate chạy cả hai.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{
    ClaimRequest, EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration,
    JobStatus, Schedule, claim_for, jobs_migration, tenant_table,
};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, Statement};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
struct SendReport {
    to: String,
}

impl Job for SendReport {
    const KIND: &'static str = "reports.send";
}

/// Bộ chạy ghi lại mọi lần chạy; `fail` bật thì lỗi.
#[derive(Default)]
struct Recorder {
    runs: Mutex<Vec<JobContext>>,
    fail: AtomicBool,
}

#[async_trait::async_trait]
impl JobHandler for Recorder {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        match job.payload["to"].as_str() {
            Some("slow") => tokio::time::sleep(std::time::Duration::from_millis(1500)).await,
            Some("panic") => panic!("the handler has a bug"),
            _ => {}
        }
        self.runs.lock().unwrap().push(job.clone());
        if self.fail.load(Ordering::SeqCst) {
            return Err(Error::internal("the mail server is down"));
        }
        Ok(())
    }
}

struct Reports {
    recorder: Arc<Recorder>,
    schedules: Vec<Schedule>,
}

impl Module for Reports {
    fn name(&self) -> &'static str {
        "reports"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            jobs_migration("m0001_create_jobs", "reports"),
            adopt("m0002_tenancy", vec![tenant_table("reports")]),
        ]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new("reports.send", self.recorder.clone())]
    }

    fn schedules(&self) -> Vec<Schedule> {
        self.schedules.clone()
    }
}

async fn setup(
    test: &TestDatabase,
    schedules: Vec<Schedule>,
) -> (Database, Modules, Arc<Recorder>) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let recorder = Arc::new(Recorder::default());
    let module = Reports { recorder: recorder.clone(), schedules };
    let mut modules = Modules::new(vec![Box::new(module)]).unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    (database, modules, recorder)
}

async fn enqueue(database: &Database, to: &str, options: EnqueueOptions) -> Option<Uuid> {
    let queue = JobQueue::new("reports");
    let tenant = TenantContext::default_on(database.clone());
    queue
        .enqueue(database.write(), &tenant, &SendReport { to: to.to_owned() }, options)
        .await
        .unwrap()
}

/// Cho thời gian trôi qua: mọi job hẹn giờ tới hạn ngay.
async fn time_passes(database: &Database) {
    let backend = database.write().get_database_backend();
    let sql = "UPDATE reports_jobs SET run_at = created_at";
    database.write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();
}

#[tokio::test]
async fn job_trong_transaction_bi_huy_thi_khong_ton_tai() {
    TestDatabase::run(async |test| {
        let (database, modules, recorder) = setup(test, Vec::new()).await;
        let queue = JobQueue::new("reports");

        let transaction = database.begin_write().await.unwrap();
        let draft = SendReport { to: "draft".to_owned() };
        queue
            .enqueue(
                &transaction,
                &TenantContext::default_on(database.clone()),
                &draft,
                EnqueueOptions::default(),
            )
            .await
            .unwrap();
        transaction.rollback().await.unwrap();
        enqueue(&database, "ada", EnqueueOptions::default()).await;

        let worker = modules.worker(4).unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        assert_eq!(worker.run_once().await.unwrap(), 0);

        let runs = recorder.runs.lock().unwrap().clone();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, "reports.send");
        assert_eq!(runs[0].payload, serde_json::json!({"to": "ada"}));
        assert_eq!(runs[0].attempt, 1);
        let done = worker.list("reports", JobStatus::Completed, 10).await.unwrap();
        assert_eq!(done.len(), 1);
        // Job xong thì dữ liệu của nó bị xoá (có thể chứa token).
        let backend = database.read().get_database_backend();
        let sql = "SELECT payload FROM reports_jobs";
        let row = database.read().query_one_raw(Statement::from_string(backend, sql));
        let payload: String = row.await.unwrap().unwrap().try_get("", "payload").unwrap();
        assert_eq!(payload, "null");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hen_gio_va_chong_trung() {
    TestDatabase::run(async |test| {
        let (database, modules, recorder) = setup(test, Vec::new()).await;
        let later = Timestamp::now() + SignedDuration::from_hours(1);
        enqueue(&database, "later", EnqueueOptions::default().run_at(later)).await;
        let unique = || EnqueueOptions::default().unique_key("daily");

        assert!(enqueue(&database, "once", unique()).await.is_some());
        assert!(enqueue(&database, "twice", unique()).await.is_none());

        let worker = modules.worker(4).unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        assert_eq!(recorder.runs.lock().unwrap()[0].payload["to"], "once");
    })
    .await
    .unwrap();
}

/// Job trùng `unique_key` GIỮA một transaction: các lệnh sau vẫn chạy và commit giữ mọi thứ.
/// Trước đây Postgres hỏng cả transaction sau lệnh chèn lỗi (D-158).
#[tokio::test]
async fn trung_khoa_giua_transaction_khong_hong_transaction() {
    TestDatabase::run(async |test| {
        let (database, modules, recorder) = setup(test, Vec::new()).await;
        let queue = JobQueue::new("reports");
        let unique = |key: &str| EnqueueOptions::default().unique_key(key);
        enqueue(&database, "first", unique("a")).await;

        let transaction = database.begin_write().await.unwrap();
        let again = SendReport { to: "again".to_owned() };
        assert!(
            queue
                .enqueue(
                    &transaction,
                    &TenantContext::default_on(database.clone()),
                    &again,
                    unique("a")
                )
                .await
                .unwrap()
                .is_none()
        );
        let next = SendReport { to: "next".to_owned() };
        assert!(
            queue
                .enqueue(
                    &transaction,
                    &TenantContext::default_on(database.clone()),
                    &next,
                    unique("b")
                )
                .await
                .unwrap()
                .is_some()
        );
        transaction.commit().await.unwrap();

        let worker = modules.worker(4).unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 2);
        let mut sent: Vec<String> = recorder
            .runs
            .lock()
            .unwrap()
            .iter()
            .map(|run| run.payload["to"].as_str().unwrap().to_owned())
            .collect();
        sent.sort();
        assert_eq!(sent, ["first", "next"]);
    })
    .await
    .unwrap();
}

/// Nhiều worker (như nhiều tiến trình) claim cùng lúc: mỗi job chạy đúng một lần.
#[tokio::test]
async fn nhieu_worker_dong_thoi_khong_chay_trung() {
    TestDatabase::run(async |test| {
        let (database, modules, recorder) = setup(test, Vec::new()).await;
        for index in 0..40 {
            enqueue(&database, &format!("user-{index}"), EnqueueOptions::default()).await;
        }

        let workers: Vec<_> = (0..3).map(|_| modules.worker(50).unwrap()).collect();
        let (a, b, c) =
            tokio::join!(workers[0].run_once(), workers[1].run_once(), workers[2].run_once());

        assert_eq!(a.unwrap() + b.unwrap() + c.unwrap(), 40);
        let mut counts: HashMap<Uuid, usize> = HashMap::new();
        for run in recorder.runs.lock().unwrap().iter() {
            *counts.entry(run.id).or_default() += 1;
        }
        assert_eq!(counts.len(), 40);
        assert!(counts.values().all(|count| *count == 1), "{counts:?}");
    })
    .await
    .unwrap();
}

/// Gọi thẳng cách claim của database đang chọn từ nhiều task: các tập job không giao nhau.
#[tokio::test]
async fn claim_dong_thoi_tra_ve_cac_tap_roi_nhau() {
    TestDatabase::run(async |test| {
        let (database, _modules, _) = setup(test, Vec::new()).await;
        for index in 0..30 {
            enqueue(&database, &format!("user-{index}"), EnqueueOptions::default()).await;
        }
        let claim: Arc<dyn autoxide_kernel::job::Claim> =
            Arc::from(claim_for(database.write().get_database_backend()));

        let tasks: Vec<_> = (0..5)
            .map(|worker| {
                let (claim, database) = (claim.clone(), database.clone());
                tokio::spawn(async move {
                    let now = Timestamp::now();
                    let until = now + SignedDuration::from_mins(5);
                    let name = format!("w{worker}");
                    claim
                        .claim(
                            database.write(),
                            &ClaimRequest::new("reports_jobs", &name, now, until, 10),
                        )
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut seen = Vec::new();
        for task in tasks {
            seen.extend(task.await.unwrap().into_iter().map(|job| job.id));
        }

        let unique: std::collections::BTreeSet<Uuid> = seen.iter().copied().collect();
        assert_eq!(unique.len(), seen.len(), "a job was claimed twice");
        assert_eq!(seen.len(), 30);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loi_thi_thu_lai_roi_vao_dead_letter_queue_va_chay_lai_duoc() {
    TestDatabase::run(async |test| {
        let (database, modules, recorder) = setup(test, Vec::new()).await;
        recorder.fail.store(true, Ordering::SeqCst);
        let options = EnqueueOptions::default().max_attempts(3);
        let id = enqueue(&database, "ada", options).await.unwrap();
        let worker = modules.worker(4).unwrap();

        for attempt in 1..=3 {
            assert_eq!(worker.run_once().await.unwrap(), 1, "attempt {attempt}");
            // Lần sau được hẹn theo backoff: chưa tới giờ thì không chạy.
            assert_eq!(worker.run_once().await.unwrap(), 0);
            time_passes(&database).await;
        }
        assert_eq!(worker.run_once().await.unwrap(), 0);

        let failed = worker.list("reports", JobStatus::Failed, 10).await.unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!((failed[0].id, failed[0].attempts), (id, 3));
        assert!(failed[0].last_error.as_deref().unwrap().contains("the mail server is down"));
        let attempts: Vec<i32> =
            recorder.runs.lock().unwrap().iter().map(|run| run.attempt).collect();
        assert_eq!(attempts, [1, 2, 3]);

        // Sửa xong nguyên nhân, người vận hành chạy lại job từ dead-letter queue.
        recorder.fail.store(false, Ordering::SeqCst);
        worker.retry("reports", id).await.unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        assert!(worker.list("reports", JobStatus::Failed, 10).await.unwrap().is_empty());
        let missing = worker.retry("reports", id).await.unwrap_err();
        assert_eq!(missing.code(), "job_not_found");
        assert_eq!(worker.retry("nope", id).await.unwrap_err().code(), "job_module_not_found");
    })
    .await
    .unwrap();
}

/// Hai worker cùng tới hạn một lịch: chỉ một job cho khoảng đó.
#[tokio::test]
async fn lich_dinh_ky_khong_sinh_trung() {
    TestDatabase::run(async |test| {
        let schedule = Schedule::new("reports.hourly", "reports.send", Duration::from_secs(3600));
        let (_database, modules, recorder) = setup(test, vec![schedule]).await;
        let first = modules.worker(4).unwrap();
        let second = modules.worker(4).unwrap();

        let (a, b) = tokio::join!(first.run_once(), second.run_once());
        a.unwrap();
        b.unwrap();
        first.run_once().await.unwrap();

        assert_eq!(recorder.runs.lock().unwrap().len(), 1);
        let pending = first.list("reports", JobStatus::Pending, 10).await.unwrap();
        assert!(pending.is_empty(), "{pending:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khai_bao_sai_bi_tu_choi_luc_khoi_dong() {
    TestDatabase::run(async |test| {
        let wrong = Schedule::new("reports.hourly", "reports.missing", Duration::from_secs(60));
        let (_database, modules, _) = setup(test, vec![wrong]).await;

        let error = modules.worker(4).err().unwrap().to_string();

        assert!(error.contains("runs job kind `reports.missing`, which has no handler"), "{error}");
    })
    .await
    .unwrap();
}

/// Worker chạy liên tục: job chậm chỉ giữ chỗ của chính nó, job khác được nhận ngay khi có chỗ
/// — không chờ cả lượt xong (D-158).
#[tokio::test]
async fn job_cham_khong_giu_cac_job_khac() {
    TestDatabase::run(async |test| {
        let (database, modules, recorder) = setup(test, Vec::new()).await;
        enqueue(&database, "slow", EnqueueOptions::default()).await;
        for name in ["a", "b", "c"] {
            enqueue(&database, name, EnqueueOptions::default()).await;
        }
        let worker = modules.worker(2).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(worker.run(async move {
            let _ = stopped.await;
        }));
        // Ba job nhanh xong trong khi job chậm (1,5 giây) còn chạy.
        let started = std::time::Instant::now();
        while recorder.runs.lock().unwrap().len() < 3 {
            assert!(started.elapsed() < std::time::Duration::from_secs(5), "quick jobs are stuck");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1400),
            "waited for the slow job"
        );
        assert!(recorder.runs.lock().unwrap().iter().all(|run| run.payload["to"] != "slow"));
        // Dừng: chờ job đang chạy xong.
        stop.send(()).unwrap();
        task.await.unwrap();
        assert_eq!(recorder.runs.lock().unwrap().len(), 4);
    })
    .await
    .unwrap();
}

/// Handler panic: ghi như một lần lỗi rồi vào dead-letter queue. Job đã chạy đủ số lần mà lần
/// cuối không ghi được kết quả (tiến trình chết): không lấy lại nữa, lượt dọn đưa vào
/// dead-letter queue (D-158).
#[tokio::test]
async fn panic_va_tien_trinh_chet_vao_dead_letter_queue() {
    TestDatabase::run(async |test| {
        let (database, modules, _) = setup(test, Vec::new()).await;
        let once = || EnqueueOptions::default().max_attempts(1);
        enqueue(&database, "panic", once()).await;
        let worker = modules.worker(4).unwrap();
        assert_eq!(worker.run_once().await.unwrap(), 1);
        let failed = worker.list("reports", JobStatus::Failed, 10).await.unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].last_error.as_deref(), Some("the job handler panicked"));

        // Chạy hết lượt mà tiến trình chết: còn giữ, hết hạn giữ, chưa ghi kết quả.
        let stuck = enqueue(&database, "ada", once()).await.unwrap();
        use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};
        let crashed = Query::update()
            .table(Alias::new("reports_jobs"))
            .value(Alias::new("attempts"), 1)
            .value(Alias::new("claimed_by"), "dead")
            .value(Alias::new("claimed_until"), Timestamp::now() - SignedDuration::from_mins(1))
            .and_where(Expr::col(Alias::new("id")).eq(stuck))
            .to_owned();
        assert_eq!(database.write().execute(&crashed).await.unwrap().rows_affected(), 1);
        let fresh = modules.worker(4).unwrap();
        assert_eq!(fresh.run_once().await.unwrap(), 0, "not claimed again");
        let failed = fresh.list("reports", JobStatus::Failed, 10).await.unwrap();
        assert!(failed.iter().any(|job| job.id == stuck), "{failed:?}");
    })
    .await
    .unwrap();
}
