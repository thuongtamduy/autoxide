//! `Throttle` đếm trong database: chặn khi vượt giới hạn, hết cửa sổ thì đếm lại, đếm đồng thời
//! không mất lần nào, bắn đồng thời không vượt giới hạn, mỗi khách đếm riêng. Chạy trên cả SQLite
//! và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus};
use autoxide_kernel::throttle::{Outcome, Throttle, tenant_table, throttle_migration};
use autoxide_testkit::TestDatabase;
use sea_orm::{ConnectionTrait, Statement};

struct Demo;

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            throttle_migration("m0001_create_throttle", "demo"),
            adopt("m0002_tenancy", vec![tenant_table("demo")]),
        ]
    }
}

async fn database(test: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    Modules::new(vec![Box::new(Demo)]).unwrap().migrator().unwrap().up(&database).await.unwrap();
    database
}

async fn sql(database: &Database, text: &str) -> i64 {
    let backend = database.read().get_database_backend();
    let row = database.read().query_one_raw(Statement::from_string(backend, text)).await.unwrap();
    row.map_or(0, |row| row.try_get_by_index::<i64>(0).unwrap())
}

#[tokio::test]
async fn vuot_gioi_han_thi_chan_roi_xoa_thi_quen() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let throttle = Throttle::new("demo", 3, Duration::from_secs(900)).unwrap();
        for _ in 0..3 {
            assert_eq!(throttle.hit(&tenant, "ip:a").await.unwrap(), Outcome::Allowed);
        }
        let Outcome::Limited { retry_after } = throttle.hit(&tenant, "ip:a").await.unwrap() else {
            panic!()
        };
        assert!((899..=900).contains(&retry_after), "{retry_after}");
        // Đang bị chặn: lần sau không đếm thêm, vẫn bị chặn.
        assert!(!throttle.hit(&tenant, "ip:a").await.unwrap().is_allowed());
        assert!(
            throttle.check(&tenant, "ip:b").await.unwrap().is_allowed(),
            "other keys are not affected"
        );
        // Khoá không lưu nguyên văn.
        assert_eq!(
            sql(&database, "SELECT COUNT(*) FROM demo_throttle WHERE key = 'ip:a'").await,
            0
        );
        throttle.clear(&tenant, "ip:a").await.unwrap();
        assert!(throttle.check(&tenant, "ip:a").await.unwrap().is_allowed());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn het_cua_so_thi_dem_lai_va_don_dep() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let throttle = Throttle::new("demo", 2, Duration::from_secs(60))
            .unwrap()
            .with_lock(Duration::from_secs(600))
            .unwrap();
        for _ in 0..3 {
            let _ = throttle.hit(&tenant, "k").await.unwrap();
        }
        let Outcome::Limited { retry_after } = throttle.check(&tenant, "k").await.unwrap() else { panic!() };
        assert!(retry_after > 60, "lock is longer than the window: {retry_after}");
        // Thời gian trôi qua: hết chặn và hết cửa sổ.
        let backend = database.write().get_database_backend();
        let past = "UPDATE demo_throttle SET locked_until = '2000-01-01 00:00:00+00:00', window_started_at = '2000-01-01 00:00:00+00:00'";
        database.write().execute_raw(Statement::from_string(backend, past)).await.unwrap();
        assert!(throttle.hit(&tenant, "k").await.unwrap().is_allowed());
        assert_eq!(sql(&database, "SELECT CAST(count AS BIGINT) FROM demo_throttle").await, 1);
        throttle.refund(&tenant, "k").await.unwrap();
        assert_eq!(sql(&database, "SELECT CAST(count AS BIGINT) FROM demo_throttle").await, 0);
        // Dọn: dòng của cửa sổ cũ, không còn chặn.
        throttle.record(&tenant, "old").await.unwrap();
        let old = "UPDATE demo_throttle SET window_started_at = '2000-01-01 00:00:00+00:00'";
        database.write().execute_raw(Statement::from_string(backend, old)).await.unwrap();
        throttle.record(&tenant, "fresh").await.unwrap();
        let removed = throttle.purge(&database, chrono::Utc::now() - chrono::Duration::days(1)).await.unwrap();
        assert_eq!(removed, 2);
        assert_eq!(sql(&database, "SELECT COUNT(*) FROM demo_throttle").await, 1);
    })
    .await
    .unwrap();
}

/// Đếm đồng thời (kể cả lần đầu, khi chưa có dòng nào): không mất lần nào.
#[tokio::test]
async fn dem_dong_thoi_du_tung_lan() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let throttle = Arc::new(Throttle::new("demo", 100, Duration::from_secs(900)).unwrap());
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let (throttle, tenant) = (throttle.clone(), tenant.clone());
                tokio::spawn(async move { throttle.record(&tenant, "k").await })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        assert_eq!(sql(&database, "SELECT CAST(count AS BIGINT) FROM demo_throttle").await, 8);
    })
    .await
    .unwrap();
}

/// Bắn đồng thời: không bao giờ quá `limit` lần được cho qua, và khoá bị chặn sau đó.
#[tokio::test]
async fn ban_dong_thoi_khong_vuot_gioi_han() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let throttle = Arc::new(Throttle::new("demo", 5, Duration::from_secs(900)).unwrap());
        let tasks: Vec<_> = (0..40)
            .map(|_| {
                let (throttle, tenant) = (throttle.clone(), tenant.clone());
                tokio::spawn(async move { throttle.hit(&tenant, "k").await })
            })
            .collect();
        let mut allowed = 0;
        for task in tasks {
            if task.await.unwrap().unwrap().is_allowed() {
                allowed += 1;
            }
        }
        assert!(allowed <= 5, "{allowed} got through");
        assert!(!throttle.check(&tenant, "k").await.unwrap().is_allowed());
    })
    .await
    .unwrap();
}

/// Hai khách cùng một khoá: đếm riêng, khách này bị chặn không làm khách kia bị chặn.
#[tokio::test]
async fn moi_khach_dem_rieng() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let ours = TenantContext::legacy_default(&database);
        let theirs = TenantContext::new(
            TenantId::from_uuid(uuid::Uuid::now_v7()),
            "other",
            "Other",
            TenantStatus::Active,
            MAIN_PLACEMENT,
            database.clone(),
        );
        let throttle = Throttle::new("demo", 1, Duration::from_secs(900)).unwrap();
        assert!(throttle.hit(&ours, "email:ada").await.unwrap().is_allowed());
        assert!(!throttle.hit(&ours, "email:ada").await.unwrap().is_allowed());
        assert!(throttle.check(&theirs, "email:ada").await.unwrap().is_allowed());
        assert!(throttle.hit(&theirs, "email:ada").await.unwrap().is_allowed());
        throttle.clear(&theirs, "email:ada").await.unwrap();
        assert!(!throttle.check(&ours, "email:ada").await.unwrap().is_allowed(), "only theirs");
    })
    .await
    .unwrap();
}
