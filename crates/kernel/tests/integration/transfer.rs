//! Chép dữ liệu giữa hai database: một file SQLite → database gate đang chọn (lượt
//! Postgres của gate là SQLite → Postgres) → một file SQLite khác (Postgres → SQLite).
//! Mọi kiểu cột module dùng, kể cả NULL, đi qua nguyên vẹn.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::transfer::{CopiedTable, copy_modules};
use autoxide_testkit::TestDatabase;
use chrono::{SubsecRound, Utc};
use sea_orm::sea_query::{Alias, Query};
use sea_orm::{ConnectionTrait, Value};
use uuid::Uuid;

struct CreateTables;

impl MigrationName for CreateTables {
    fn name(&self) -> &str {
        "m0001_create_tables"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTables {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Bảng con tạo SAU bảng cha (như migration thật): chép theo thứ tự tạo thì
        // foreign key luôn thoả.
        manager
            .create_table(
                Table::create()
                    .table("shop_item")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("name", 100))
                    .col(schema::text_null("note"))
                    .col(schema::integer("quantity"))
                    .col(schema::big_integer("price_minor"))
                    .col(schema::boolean("active"))
                    .col(schema::blob_null("thumbnail"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("deleted_at"))
                    .col(schema::uuid_null("owner_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("shop_tag")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("item_id"))
                    .col(schema::string("label"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("shop_tag_item_fk")
                            .from("shop_tag", "item_id")
                            .to("shop_item", "id"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("shop_tag").to_owned()).await?;
        manager.drop_table(Table::drop().table("shop_item").to_owned()).await
    }
}

fn migrator() -> Migrator {
    Migrator::new(vec![ModuleMigrations::new("shop", vec![Box::new(CreateTables)])]).unwrap()
}

async fn connect(url: &str) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", url)]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    let database = Database::connect(&reader.finish(config).unwrap().0).await.unwrap();
    migrator().up(&database).await.unwrap();
    database
}

/// Một file SQLite mới trong thư mục tạm riêng.
async fn sqlite(directory: &std::path::Path, name: &str) -> Database {
    let path = directory.join(name);
    connect(&format!("sqlite://{}?mode=rwc", path.display())).await
}

/// Mọi dòng của hai bảng, đọc theo kiểu, sắp theo id.
async fn dump(database: &Database) -> Vec<Vec<Value>> {
    let mut rows = Vec::new();
    for (table, columns) in [
        (
            "shop_item",
            &[
                "id",
                "name",
                "note",
                "quantity",
                "price_minor",
                "active",
                "thumbnail",
                "created_at",
                "deleted_at",
                "owner_id",
            ][..],
        ),
        ("shop_tag", &["id", "item_id", "label"][..]),
    ] {
        let mut select = Query::select();
        select.from(Alias::new(table)).order_by(Alias::new("id"), sea_orm::Order::Asc);
        for column in columns {
            select.column(Alias::new(*column));
        }
        let backend = database.read().get_database_backend();
        for row in database.read().query_all_raw(backend.build(&select)).await.unwrap() {
            let mut values = Vec::new();
            for column in columns {
                values.push(match *column {
                    "id" | "item_id" => Value::Uuid(Some(row.try_get("", column).unwrap())),
                    "owner_id" => Value::Uuid(row.try_get("", column).unwrap()),
                    "name" | "label" => Value::String(Some(row.try_get("", column).unwrap())),
                    "note" => Value::String(row.try_get("", column).unwrap()),
                    "quantity" => Value::Int(Some(row.try_get("", column).unwrap())),
                    "price_minor" => Value::BigInt(Some(row.try_get("", column).unwrap())),
                    "active" => Value::Bool(Some(row.try_get("", column).unwrap())),
                    "thumbnail" => Value::Bytes(row.try_get("", column).unwrap()),
                    "created_at" => {
                        Value::ChronoDateTimeUtc(Some(row.try_get("", column).unwrap()))
                    }
                    "deleted_at" => Value::ChronoDateTimeUtc(row.try_get("", column).unwrap()),
                    other => panic!("unknown column {other}"),
                });
            }
            rows.push(values);
        }
    }
    rows
}

async fn fill(database: &Database) {
    let now = Utc::now().trunc_subsecs(6);
    for index in 0..250i64 {
        let id = Uuid::now_v7();
        let mut insert = Query::insert();
        insert
            .into_table(Alias::new("shop_item"))
            .columns(
                [
                    "id",
                    "name",
                    "note",
                    "quantity",
                    "price_minor",
                    "active",
                    "thumbnail",
                    "created_at",
                    "deleted_at",
                    "owner_id",
                ]
                .map(Alias::new),
            )
            .values_panic([
                id.into(),
                format!("Item {index} — “ngoặc” ✓").into(),
                (index % 2 == 0).then(|| format!("note {index}")).into(),
                <i32 as TryFrom<i64>>::try_from(index).unwrap().into(),
                (index * 1_000_000_007).into(),
                (index % 3 == 0).into(),
                (index % 5 == 0)
                    .then(|| {
                        vec![0u8, 255, 7, <u8 as TryFrom<i64>>::try_from(index % 256).unwrap()]
                    })
                    .into(),
                (now - chrono::Duration::seconds(index)).into(),
                (index % 7 == 0).then_some(now).into(),
                (index % 4 == 0).then(Uuid::now_v7).into(),
            ]);
        database.write().execute(&insert).await.unwrap();
        let mut tag = Query::insert();
        tag.into_table(Alias::new("shop_tag"))
            .columns(["id", "item_id", "label"].map(Alias::new))
            .values_panic([Uuid::now_v7().into(), id.into(), format!("tag {index}").into()]);
        database.write().execute(&tag).await.unwrap();
    }
}

#[tokio::test]
async fn chep_qua_lai_giu_nguyen_moi_kieu() {
    TestDatabase::run(async |test| {
        let directory = std::env::temp_dir().join(format!("autoxide-transfer-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&directory).unwrap();
        let first = sqlite(&directory, "first.sqlite").await;
        fill(&first).await;
        let middle = connect(test.url()).await;
        let last = sqlite(&directory, "last.sqlite").await;

        let copied = copy_modules(&first, &middle, &["shop"]).await.unwrap();
        assert_eq!(
            copied,
            [CopiedTable::new("shop_item", 250), CopiedTable::new("shop_tag", 250),]
        );
        copy_modules(&middle, &last, &["shop"]).await.unwrap();

        let original = dump(&first).await;
        assert_eq!(original.len(), 500);
        assert_eq!(dump(&middle).await, original);
        assert_eq!(dump(&last).await, original);

        // Chép lại vào đích đã có dữ liệu: dữ liệu cũ bị thay, không nhân đôi.
        copy_modules(&first, &middle, &["shop"]).await.unwrap();
        assert_eq!(dump(&middle).await, original);
        for database in [first, middle, last] {
            database.close().await.unwrap();
        }
        std::fs::remove_dir_all(&directory).unwrap();
    })
    .await
    .unwrap();
}
