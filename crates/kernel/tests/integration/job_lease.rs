//! Job chạy lâu hơn hạn giữ (bước 39, A1): worker gia hạn giữ trong lúc chạy nên worker khác không
//! lấy lại và chạy chồng; mất quyền giữ thì job bị dừng. Chạy trên database gate đang chọn.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::job::{
    EnqueueOptions, Job, JobContext, JobHandler, JobQueue, JobRegistration, jobs_migration,
    tenant_table,
};
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, Statement};
use serde::Serialize;

#[derive(Serialize)]
struct Slow;

impl Job for Slow {
    const KIND: &'static str = "slow.run";
}

/// Chạy `seconds` giây; đếm lần bắt đầu và lần chạy xong.
#[derive(Default)]
struct Sleeper {
    started: AtomicUsize,
    finished: AtomicUsize,
}

#[async_trait::async_trait]
impl JobHandler for Sleeper {
    async fn run(&self, _job: &JobContext) -> Result<(), Error> {
        self.started.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(4)).await;
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct SlowModule(Arc<Sleeper>);

impl Module for SlowModule {
    fn name(&self) -> &'static str {
        "slow"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            jobs_migration("m0001_create_jobs", "slow"),
            adopt("m0002_tenancy", vec![tenant_table("slow")]),
        ]
    }

    fn jobs(&self) -> Vec<JobRegistration> {
        vec![JobRegistration::new("slow.run", self.0.clone()).timeout(Duration::from_secs(30))]
    }
}

async fn setup(test: &TestDatabase) -> (Database, Modules, Arc<Sleeper>) {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    let sleeper = Arc::new(Sleeper::default());
    let mut modules = Modules::new(vec![Box::new(SlowModule(sleeper.clone()))]).unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let tenant = TenantContext::default_on(database.clone());
    JobQueue::new("slow")
        .enqueue(database.write(), &tenant, &Slow, EnqueueOptions::default())
        .await
        .unwrap();
    (database, modules, sleeper)
}

#[tokio::test]
async fn job_dai_duoc_gia_han_lease() {
    TestDatabase::run(async |test| {
        let (_database, modules, sleeper) = setup(test).await;
        // Hạn giữ 1,5 giây, job chạy 4 giây: không gia hạn thì worker thứ hai lấy lại được.
        let first = modules.worker(1).unwrap().lease(Duration::from_millis(1500));
        let second = modules.worker(1).unwrap().lease(Duration::from_millis(1500));
        let running = tokio::spawn(async move { first.run_once().await.unwrap() });
        let mut stolen = 0;
        for _ in 0..6 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            stolen += second.run_once().await.unwrap();
        }
        assert_eq!(running.await.unwrap(), 1);
        assert_eq!(stolen, 0, "the second worker must not take the job while it runs");
        assert_eq!(sleeper.started.load(Ordering::SeqCst), 1);
        assert_eq!(sleeper.finished.load(Ordering::SeqCst), 1);
        // Đã xong: không còn gì để chạy.
        assert_eq!(second.run_once().await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn job_mat_quyen_giu_thi_dung() {
    TestDatabase::run(async |test| {
        let (database, modules, sleeper) = setup(test).await;
        let worker = modules.worker(1).unwrap().lease(Duration::from_millis(1500));
        let running = tokio::spawn(async move { worker.run_once().await.unwrap() });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(sleeper.started.load(Ordering::SeqCst), 1);
        // Ai đó (lệnh vận hành, worker khác sau khi hết hạn) lấy job đi.
        let backend = database.write().get_database_backend();
        let sql = "UPDATE slow_jobs SET claimed_by = 'someone-else'";
        database.write().execute_raw(Statement::from_string(backend, sql)).await.unwrap();

        running.await.unwrap();
        // Worker dừng job ở lần gia hạn kế tiếp, trước khi nó chạy xong 4 giây.
        assert_eq!(sleeper.finished.load(Ordering::SeqCst), 0);
        let check = "SELECT claimed_by FROM slow_jobs WHERE completed_at IS NULL";
        let row = database
            .read()
            .query_one_raw(Statement::from_string(backend, check))
            .await
            .unwrap()
            .unwrap();
        // Không ghi đè dòng của bên đã lấy job.
        assert_eq!(row.try_get::<String>("", "claimed_by").unwrap(), "someone-else");
    })
    .await
    .unwrap();
}
