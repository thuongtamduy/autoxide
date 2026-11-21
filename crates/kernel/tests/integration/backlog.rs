//! Số đo tồn đọng (bước 39, A3) trên database gate đang chọn: đếm job đang chờ, tuổi của job tới
//! hạn chờ lâu nhất, dead-letter queue, event chưa giao — cùng câu truy vấn trên SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::{Arc, Mutex};

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox, outbox_migration};
use autoxide_kernel::job::backlog::{ModuleBacklog, backlog};
use autoxide_kernel::job::{
    EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration, jobs_migration,
    tenant_table,
};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, Statement};
use serde::Serialize;

#[derive(Serialize)]
struct SendReport;

impl Job for SendReport {
    const KIND: &'static str = "reports.send";
}

#[derive(Serialize)]
struct ReportSent;

impl Event for ReportSent {
    const NAME: &'static str = "reports.sent";
}

struct Nothing;

#[async_trait::async_trait]
impl JobHandler for Nothing {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        Ok(())
    }
}

/// Module có cả bảng job lẫn bảng event.
struct Reports(Arc<Mutex<Option<Outbox>>>);

impl Module for Reports {
    fn name(&self) -> &'static str {
        "reports"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            jobs_migration("m0001_create_jobs", "reports"),
            outbox_migration("m0002_create_events", "reports"),
            autoxide_kernel::event::outbox_actor_migration("m0003_add_event_actor", "reports"),
            adopt(
                "m0004_tenancy",
                vec![tenant_table("reports"), autoxide_kernel::event::tenant_table("reports")],
            ),
            autoxide_kernel::event::outbox_access_migration("k0001_event_access", "reports"),
        ]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new("reports.send", Arc::new(Nothing))]
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![EventInfo::new("reports.sent", "A report was sent.")]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(context.outbox()?);
        Ok(())
    }
}

async fn setup(test: &TestDatabase) -> (Database, Modules, Outbox) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let slot = Arc::new(Mutex::new(None));
    let mut modules = Modules::new(vec![Box::new(Reports(slot.clone()))]).unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let outbox = slot.lock().unwrap().clone().unwrap();
    (database, modules, outbox)
}

async fn enqueue(database: &Database, options: EnqueueOptions) {
    let tenant = TenantContext::default_on(database.clone());
    JobQueue::new("reports")
        .enqueue(database.write(), &tenant, &SendReport, options)
        .await
        .unwrap();
}

async fn execute(database: &Database, sql: &str) {
    let backend = database.write().get_database_backend();
    database.write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();
}

async fn reports(database: &Database, now: Timestamp) -> ModuleBacklog {
    let snapshot = backlog(database, "main", &["reports"], &["reports"], now).await.unwrap();
    assert_eq!(snapshot.database, "main");
    assert_eq!(snapshot.pools.iter().map(|pool| pool.pool).collect::<Vec<_>>(), ["read", "write"]);
    assert!(snapshot.pools.iter().all(|pool| pool.max >= 1 && pool.in_use + pool.idle <= pool.max));
    snapshot.modules.into_iter().find(|found| found.module == "reports").unwrap()
}

#[tokio::test]
async fn ton_dong_dem_job_cho_tuoi_va_dead_letter() {
    TestDatabase::run(async |test| {
        let (database, _modules, _outbox) = setup(test).await;
        let empty = reports(&database, Timestamp::now()).await;
        assert_eq!(
            (empty.jobs_pending, empty.jobs_oldest_pending_seconds, empty.jobs_dead),
            (0, 0, 0)
        );

        for _ in 0..3 {
            enqueue(&database, EnqueueOptions::default()).await;
        }
        // Một job hỏng hẳn: vào dead-letter queue, không còn tính là đang chờ.
        execute(
            &database,
            "UPDATE reports_jobs SET failed_at = created_at WHERE id = \
             (SELECT id FROM reports_jobs ORDER BY id LIMIT 1)",
        )
        .await;
        // Mười phút sau.
        let later = Timestamp::now() + SignedDuration::from_mins(10);
        let found = reports(&database, later).await;
        assert_eq!((found.jobs_pending, found.jobs_dead), (2, 1));
        assert!(
            (600..=660).contains(&found.jobs_oldest_pending_seconds),
            "{}",
            found.jobs_oldest_pending_seconds
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ton_dong_khong_dem_job_hen_gio_chua_toi() {
    TestDatabase::run(async |test| {
        let (database, _modules, _outbox) = setup(test).await;
        let tomorrow = Timestamp::now() + SignedDuration::from_hours(24);
        enqueue(&database, EnqueueOptions::default().run_at(tomorrow)).await;

        let found = reports(&database, Timestamp::now()).await;
        // Đang chờ (sẽ chạy), nhưng chưa tới hạn nên chưa "chờ lâu".
        assert_eq!((found.jobs_pending, found.jobs_oldest_pending_seconds), (1, 0));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ton_dong_dem_event_chua_giao() {
    TestDatabase::run(async |test| {
        let (database, _modules, outbox) = setup(test).await;
        for _ in 0..2 {
            let transaction = database.begin_write().await.unwrap();
            let tenant = TenantContext::default_on(database.clone());
            outbox.record(&transaction, &tenant, Actor::system(), None, &ReportSent).await.unwrap();
            transaction.commit().await.unwrap();
        }
        execute(
            &database,
            "UPDATE reports_events SET failed_at = occurred_at WHERE id = \
             (SELECT id FROM reports_events ORDER BY id LIMIT 1)",
        )
        .await;

        let later = Timestamp::now() + SignedDuration::from_mins(5);
        let found = reports(&database, later).await;
        assert_eq!((found.events_pending, found.events_dead), (1, 1));
        assert!(
            (300..=360).contains(&found.events_oldest_pending_seconds),
            "{}",
            found.events_oldest_pending_seconds
        );
    })
    .await
    .unwrap();
}
