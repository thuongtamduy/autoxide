//! Bản xuất dữ liệu của khách (`TenantTables::export`, bước 35): mỗi kiểu cột ra JSON đúng dạng — thời điểm RFC
//! 3339, ngày `YYYY-MM-DD`, NULL là `null` — chỉ dòng của khách được xuất, cột bí mật bị bỏ.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::tenancy::TenantId;
use autoxide_kernel::time::{Date, Timestamp};
use autoxide_kernel::transfer::{CopiedTable, TenantTables};
use autoxide_testkit::TestDatabase;
use sea_orm::ConnectionTrait;
use sea_orm::sea_query::{Alias, Query};
use serde_json::{Value, json};
use uuid::Uuid;

struct CreateTable;

impl MigrationName for CreateTable {
    fn name(&self) -> &str {
        "m0001_create_entry"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("keep_entry")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("name", 50))
                    .col(schema::timestamp_with_time_zone("at"))
                    .col(schema::date("day"))
                    .col(schema::date_null("maybe_day"))
                    .col(schema::string_len_null("token_hash", 100))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("keep_entry").to_owned()).await
    }
}

async fn connect(url: &str) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    Migrator::new(vec![ModuleMigrations::new("keep", vec![Box::new(CreateTable)])])
        .unwrap()
        .up(&database)
        .await
        .unwrap();
    database
}

async fn insert(database: &Database, tenant: Uuid, id: Uuid, maybe_day: Option<Date>) {
    let at: Timestamp = "2026-11-21T10:00:00.123456Z".parse().unwrap();
    let day: Date = "2026-11-21".parse().unwrap();
    let statement = Query::insert()
        .into_table(Alias::new("keep_entry"))
        .columns(
            ["id", "tenant_id", "name", "at", "day", "maybe_day", "token_hash"].map(Alias::new),
        )
        .values_panic([
            id.into(),
            tenant.into(),
            "Ada".into(),
            at.into(),
            day.into(),
            maybe_day.into(),
            "secret".into(),
        ])
        .to_owned();
    database.write().execute(&statement).await.unwrap();
}

#[tokio::test]
async fn ban_xuat_giu_ngay_thoi_diem_va_null() {
    TestDatabase::run(async |test| {
        let database = connect(test.url()).await;
        let tenant = Uuid::now_v7();
        let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
        insert(&database, tenant, first, Some("2026-12-01".parse().unwrap())).await;
        insert(&database, tenant, second, None).await;
        insert(&database, Uuid::now_v7(), Uuid::now_v7(), None).await;

        let tables = TenantTables::read(&database, &database, &["keep"], &[]).await.unwrap();
        let (sink, mut receiver) = tokio::sync::mpsc::channel(8);
        let counted = tables.export(&database, TenantId::from_uuid(tenant), &sink).await.unwrap();
        drop(sink);
        assert_eq!(counted, [CopiedTable::new("keep_entry", 2)]);

        let mut text = Vec::new();
        while let Some(chunk) = receiver.recv().await {
            text.extend_from_slice(&chunk.unwrap());
        }
        let mut lines: Vec<Value> = String::from_utf8(text)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        lines.sort_by_key(|line| line["row"]["id"].as_str().unwrap().to_owned());
        let row = |id: Uuid, maybe_day: Value| {
            json!({"table": "keep_entry", "row": {
                "id": id.to_string(),
                "tenant_id": tenant.to_string(),
                "name": "Ada",
                "at": "2026-11-21T10:00:00.123456+00:00",
                "day": "2026-11-21",
                "maybe_day": maybe_day,
            }})
        };
        // `token_hash` là cột bí mật: không có trong bản xuất.
        assert_eq!(lines, [row(first, json!("2026-12-01")), row(second, Value::Null)]);
    })
    .await
    .unwrap();
}
