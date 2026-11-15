//! Token dùng một lần: chỉ dùng được một lần, đúng mục đích, còn hạn; dùng đồng thời thì đúng
//! một bên thắng; thu hồi, liệt kê gần đây, dọn dẹp. Chạy trên cả SQLite và Postgres.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::MigrationTrait;
use autoxide_kernel::module::{Module, Modules};
use autoxide_kernel::tenancy::migration::adopt;
use autoxide_kernel::tenancy::{MAIN_PLACEMENT, TenantContext, TenantId, TenantStatus};
use autoxide_kernel::tokens::{OneTimeTokens, tenant_table, tokens_migration};
use autoxide_testkit::TestDatabase;
use chrono::{Duration, Utc};
use sea_orm::{ConnectionTrait, Statement};

struct Demo;

impl Module for Demo {
    fn name(&self) -> &'static str {
        "demo"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![
            tokens_migration("m0001_create_tokens", "demo"),
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

#[tokio::test]
async fn dung_mot_lan_dung_muc_dich_con_han() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let tokens = OneTimeTokens::new("demo");
        let write = database.write();
        let issued = tokens
            .issue(write, &tenant, "confirm", "sub-1", Some("a@example.com"), Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(issued.token.len(), 43);
        assert!(
            tokens.consume(write, &tenant, "other", &issued.token).await.unwrap().is_none(),
            "wrong purpose"
        );
        let used = tokens.consume(write, &tenant, "confirm", &issued.token).await.unwrap().unwrap();
        assert_eq!(
            (used.id, used.subject.as_str(), used.data.as_deref()),
            (issued.id, "sub-1", Some("a@example.com"))
        );
        assert!(
            tokens.consume(write, &tenant, "confirm", &issued.token).await.unwrap().is_none(),
            "used once"
        );
        assert!(tokens.consume(write, &tenant, "confirm", "nonsense").await.unwrap().is_none());

        let expired = tokens
            .issue(write, &tenant, "confirm", "sub-1", None, Duration::seconds(-1))
            .await
            .unwrap();
        assert!(
            tokens.consume(write, &tenant, "confirm", &expired.token).await.unwrap().is_none(),
            "expired"
        );
        // Token không lưu nguyên văn.
        let backend = write.get_database_backend();
        let raw = format!("SELECT COUNT(*) FROM demo_tokens WHERE token_hash = '{}'", issued.token);
        let row = write.query_one_raw(Statement::from_string(backend, raw)).await.unwrap().unwrap();
        assert_eq!(row.try_get_by_index::<i64>(0).unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gan_day_thu_hoi_va_don_dep() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let tokens = OneTimeTokens::new("demo");
        let write = database.write();
        let first = tokens
            .issue(write, &tenant, "verify", "user-1", Some("a@example.com"), Duration::hours(1))
            .await
            .unwrap();
        tokens
            .issue(write, &tenant, "verify", "user-1", Some("b@example.com"), Duration::hours(1))
            .await
            .unwrap();
        tokens.issue(write, &tenant, "verify", "user-2", None, Duration::hours(1)).await.unwrap();
        let recent = tokens
            .recent(write, &tenant, "verify", "user-1", Utc::now() - Duration::minutes(1))
            .await
            .unwrap();
        let data: Vec<_> = recent.iter().map(|token| token.data.as_deref()).collect();
        assert_eq!(data, [Some("b@example.com"), Some("a@example.com")]);
        assert!(
            tokens
                .recent(write, &tenant, "verify", "user-1", Utc::now() + Duration::minutes(1))
                .await
                .unwrap()
                .is_empty()
        );

        assert_eq!(tokens.revoke(write, &tenant, "verify", "user-1").await.unwrap(), 2);
        let other = tokens
            .issue(write, &tenant, "other", "user-3", None, Duration::hours(1))
            .await
            .unwrap();
        assert_eq!(tokens.forget(write, &tenant, "user-3").await.unwrap(), 1);
        assert!(tokens.consume(write, &tenant, "other", &other.token).await.unwrap().is_none());
        assert!(tokens.consume(write, &tenant, "verify", &first.token).await.unwrap().is_none());
        // Dọn theo hạn (cột có index, D-158): chưa token nào hết hạn thì không xoá gì; qua hạn
        // thì xoá cả token đã dùng (thu hồi) lẫn token chưa dùng.
        assert_eq!(tokens.purge(write, Utc::now() + Duration::minutes(1)).await.unwrap(), 0);
        assert_eq!(tokens.purge(write, Utc::now() + Duration::hours(2)).await.unwrap(), 3);
    })
    .await
    .unwrap();
}

/// Nhiều request cùng dùng một token: đúng một bên thắng.
#[tokio::test]
async fn dung_dong_thoi_mot_ben_thang() {
    TestDatabase::run(async |test| {
        let database = database(test).await;
        let tenant = TenantContext::legacy_default(&database);
        let tokens = Arc::new(OneTimeTokens::new("demo"));
        let issued = tokens
            .issue(database.write(), &tenant, "confirm", "s", None, Duration::hours(1))
            .await
            .unwrap();
        let tasks: Vec<_> = (0..12)
            .map(|_| {
                let (tokens, database, token, tenant) =
                    (tokens.clone(), database.clone(), issued.token.clone(), tenant.clone());
                tokio::spawn(async move {
                    tokens.consume(database.write(), &tenant, "confirm", &token).await
                })
            })
            .collect();
        let mut winners = 0;
        for task in tasks {
            if task.await.unwrap().unwrap().is_some() {
                winners += 1;
            }
        }
        assert_eq!(winners, 1);
    })
    .await
    .unwrap();
}

/// Token của khách này dùng ở khách khác: như token lạ (không tiêu, không lộ).
#[tokio::test]
async fn token_cua_khach_khac_nhu_token_la() {
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
        let tokens = OneTimeTokens::new("demo");
        let write = database.write();
        let issued =
            tokens.issue(write, &ours, "confirm", "s", None, Duration::hours(1)).await.unwrap();
        assert!(tokens.consume(write, &theirs, "confirm", &issued.token).await.unwrap().is_none());
        assert_eq!(tokens.revoke(write, &theirs, "confirm", "s").await.unwrap(), 0);
        assert!(tokens.consume(write, &ours, "confirm", &issued.token).await.unwrap().is_some());
    })
    .await
    .unwrap();
}
