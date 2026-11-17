//! Đánh số tuần tự không nhảy số (bước 36): cấp số đồng thời vẫn liên tục 1..n, rollback không tiêu
//! số, mỗi dãy và mỗi khách đếm riêng. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::module::Modules;
use autoxide_kernel::numbering;
use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::TestDatabase;
use uuid::Uuid;

/// Database đã chạy migration của kernel (có `kernel_sequences`).
async fn database(test: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    Modules::new(vec![]).unwrap().migrator().unwrap().up(&database).await.unwrap();
    database
}

fn tenant() -> TenantId {
    TenantId::from_uuid(Uuid::now_v7())
}

/// Một số trong transaction riêng, commit ngay.
async fn take(database: &Database, tenant: TenantId, scope: &str) -> i64 {
    let transaction = database.begin_write().await.unwrap();
    let number = numbering::next(&transaction, tenant, scope).await.unwrap();
    transaction.commit().await.unwrap();
    number
}

/// 32 transaction cùng cấp số một dãy: nhận đúng 1..32, không trùng, không bỏ số.
#[tokio::test]
async fn numbering_concurrent_next_is_gapless() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = tenant();
        let tasks: Vec<_> = (0..32)
            .map(|_| {
                let database = database.clone();
                tokio::spawn(async move {
                    database
                        .write_with_retry(|| async {
                            let transaction = database.begin_write().await?;
                            let number =
                                numbering::next(&transaction, tenant, "invoices:2026").await?;
                            transaction.commit().await?;
                            Ok(number)
                        })
                        .await
                })
            })
            .collect();
        let mut numbers = Vec::new();
        for task in tasks {
            numbers.push(task.await.unwrap().unwrap());
        }
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=32).collect::<Vec<i64>>());
        assert_eq!(numbering::current(database.read(), tenant, "invoices:2026").await.unwrap(), 32);
    })
    .await
    .unwrap();
}

/// Transaction rollback thì số không bị tiêu: lần sau nhận lại đúng số đó.
#[tokio::test]
async fn numbering_rollback_does_not_consume() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = tenant();
        assert_eq!(take(&database, tenant, "receipts").await, 1);
        let transaction = database.begin_write().await.unwrap();
        assert_eq!(numbering::next(&transaction, tenant, "receipts").await.unwrap(), 2);
        transaction.rollback().await.unwrap();
        assert_eq!(numbering::current(database.read(), tenant, "receipts").await.unwrap(), 1);
        assert_eq!(take(&database, tenant, "receipts").await, 2);
    })
    .await
    .unwrap();
}

/// Mỗi dãy, mỗi khách một bộ đếm riêng; dãy chưa cấp số nào có số hiện tại 0.
#[tokio::test]
async fn numbering_scopes_and_tenants_independent() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let (a, b) = (tenant(), tenant());
        assert_eq!(numbering::current(database.read(), a, "x").await.unwrap(), 0);
        assert_eq!(take(&database, a, "x").await, 1);
        assert_eq!(take(&database, a, "x").await, 2);
        assert_eq!(take(&database, a, "y").await, 1);
        assert_eq!(take(&database, b, "x").await, 1);
        assert_eq!(numbering::current(database.read(), a, "x").await.unwrap(), 2);
        // Tên dãy rỗng là lỗi của code gọi.
        let transaction = database.begin_write().await.unwrap();
        assert!(numbering::next(&transaction, a, "").await.is_err());
        assert!(numbering::next(&transaction, a, &"s".repeat(201)).await.is_err());
    })
    .await
    .unwrap();
}
