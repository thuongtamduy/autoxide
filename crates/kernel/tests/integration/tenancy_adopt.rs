//! Helper nâng bảng `tenancy::migration::adopt` (bước 33 A3) trên bảng mẫu, cả SQLite và
//! Postgres: giữ dữ liệu và gán khách mặc định, đổi khoá chính, giữ khoá ngoại (kể cả xoá dây
//! chuyền), từ chối khi quên index cũ, `down` từ chối khi còn khách khác, up → down → up.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{Migrator, ModuleMigrations};
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_kernel::tenancy::migration::{TenantTable, adopt};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestDatabase, schema_snapshot};
use sea_orm::{ConnectionTrait, Statement};
use uuid::Uuid;

/// Bảng mẫu trước khi có khách: cha (`id`, `name` unique), con (khoá ngoại xoá dây chuyền, unique
/// hai cột, index thời gian), bảng khoá-giá trị (khoá chính là chữ).
struct CreateDemo;

impl MigrationName for CreateDemo {
    fn name(&self) -> &str {
        "m0001_create_demo"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateDemo {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("demo_parent")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("name", 50))
                    .col(schema::integer("rank").default(0))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("demo_parent_name_unique")
                    .table("demo_parent")
                    .col("name")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("demo_child")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("parent_id"))
                    .col(schema::string_len("code", 20))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .from("demo_child", "parent_id")
                            .to("demo_parent", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        for (name, columns, unique) in [
            ("demo_child_code_unique", vec!["parent_id", "code"], true),
            ("demo_child_created_idx", vec!["created_at"], false),
        ] {
            let mut index = Index::create();
            index.name(name).table("demo_child");
            for column in columns {
                index.col(column);
            }
            if unique {
                index.unique();
            }
            manager.create_index(index.to_owned()).await?;
        }
        manager
            .create_table(
                Table::create()
                    .table("demo_kv")
                    .col(schema::string_len("key", 64).primary_key())
                    .col(schema::text("value"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["demo_kv", "demo_child", "demo_parent"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

fn adoption() -> Box<dyn MigrationTrait> {
    adopt(
        "m0002_tenancy",
        vec![
            TenantTable::new("demo_parent")
                .unique("demo_parent_name_unique", ["tenant_id", "name"]),
            TenantTable::new("demo_child")
                .unique("demo_child_code_unique", ["tenant_id", "parent_id", "code"])
                .sweep_index("demo_child_created_sweep_idx", ["created_at"])
                .drop_index("demo_child_created_idx"),
            TenantTable::new("demo_kv").primary_key(["tenant_id", "key"]),
        ],
    )
}

fn migrator(steps: Vec<Box<dyn MigrationTrait>>) -> Migrator {
    Migrator::new(vec![ModuleMigrations::new("demo", steps)]).unwrap()
}

async fn connect(test: &TestDatabase) -> Database {
    let source = autoxide_kernel::config::ConfigSource::from_pairs([("DATABASE_URL", test.url())]);
    let mut reader = autoxide_kernel::config::ConfigReader::new(&source);
    let config = autoxide_kernel::config::Config::read(&mut reader);
    let (config, _) = reader.finish(config).unwrap();
    Database::connect(&config.database).await.unwrap()
}

async fn run(database: &Database, sql: &str, values: Vec<sea_orm::Value>) {
    let backend = database.write().get_database_backend();
    let sql = if backend == sea_orm::DbBackend::Postgres {
        // `?` → `$1`, `$2`… theo thứ tự.
        let mut out = String::new();
        let mut n = 0;
        for c in sql.chars() {
            if c == '?' {
                n += 1;
                out.push_str(&format!("${n}"));
            } else {
                out.push(c);
            }
        }
        out
    } else {
        sql.to_owned()
    };
    database
        .write()
        .execute_raw(Statement::from_sql_and_values(backend, sql, values))
        .await
        .unwrap();
}

async fn count(database: &Database, sql: &str) -> i64 {
    let backend = database.read().get_database_backend();
    database
        .read()
        .query_one_raw(Statement::from_string(backend, sql))
        .await
        .unwrap()
        .unwrap()
        .try_get_by_index::<i64>(0)
        .unwrap()
}

/// Bảng mẫu có mỗi bảng một dòng; trả database, ID của cha và schema trước khi nâng.
async fn seeded(test: &TestDatabase) -> (Database, Uuid, autoxide_testkit::SchemaSnapshot) {
    let database = connect(test).await;
    migrator(vec![Box::new(CreateDemo)]).up(&database).await.unwrap();
    let parent = Uuid::now_v7();
    run(
        &database,
        "INSERT INTO demo_parent (id, name) VALUES (?, ?)",
        vec![parent.into(), "Ada".into()],
    )
    .await;
    run(
        &database,
        "INSERT INTO demo_child (id, parent_id, code, created_at) VALUES (?, ?, ?, ?)",
        vec![Uuid::now_v7().into(), parent.into(), "x".into(), Timestamp::now().into()],
    )
    .await;
    run(&database, "INSERT INTO demo_kv (key, value) VALUES (?, ?)", vec!["k".into(), "v".into()])
        .await;
    let before = schema_snapshot(database.write()).await.unwrap();
    (database, parent, before)
}

fn full() -> Migrator {
    migrator(vec![Box::new(CreateDemo), adoption()])
}

/// Bảng `demo_*` (trừ bảng lịch sử): tên và cột (tên, nullable).
fn demo_tables(snapshot: &autoxide_testkit::SchemaSnapshot) -> Vec<(String, Vec<(String, bool)>)> {
    snapshot
        .tables
        .iter()
        .filter(|table| table.name.starts_with("demo_") && !table.name.ends_with("_migrations"))
        .map(|table| {
            (
                table.name.clone(),
                table.columns.iter().map(|c| (c.name.clone(), c.nullable)).collect(),
            )
        })
        .collect()
}

#[tokio::test]
async fn adopt_giu_du_lieu_va_gan_khach_mac_dinh() {
    TestDatabase::run(async |test| {
        let (database, _, before) = seeded(test).await;
        full().up(&database).await.unwrap();
        for table in ["demo_parent", "demo_child", "demo_kv"] {
            assert_eq!(
                count(&database, &format!("SELECT COUNT(*) FROM {table}")).await,
                1,
                "{table}"
            );
        }
        let backend = database.read().get_database_backend();
        let row = database
            .read()
            .query_one_raw(Statement::from_string(backend, "SELECT tenant_id FROM demo_kv"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by_index::<Uuid>(0).unwrap(), DEFAULT_TENANT.as_uuid());
        let adopted = schema_snapshot(database.write()).await.unwrap();
        let kv = adopted.tables.iter().find(|table| table.name == "demo_kv").unwrap();
        assert_eq!(kv.columns[0].name, "tenant_id");
        assert!(!kv.columns[0].nullable && kv.columns[0].default.is_none(), "no DEFAULT");
        let child = adopted.tables.iter().find(|table| table.name == "demo_child").unwrap();
        let names: Vec<&str> = child.indexes.iter().map(|index| index.name.as_str()).collect();
        assert!(
            names.contains(&"demo_child_code_unique")
                && names.contains(&"demo_child_created_sweep_idx"),
            "{names:?}"
        );
        assert!(!names.contains(&"demo_child_created_idx"));
        // `INSERT` quên `tenant_id`: lỗi NOT NULL (không có DEFAULT âm thầm).
        let backend = database.write().get_database_backend();
        assert!(
            database
                .write()
                .execute_raw(Statement::from_string(
                    backend,
                    "INSERT INTO demo_kv (key, value) VALUES ('z', 'z')"
                ))
                .await
                .is_err()
        );
        // up → down → up: schema về như cũ rồi lại như sau khi nâng.
        full().down(&database, "demo", 1).await.unwrap();
        assert_eq!(
            demo_tables(&schema_snapshot(database.write()).await.unwrap()),
            demo_tables(&before)
        );
        full().up(&database).await.unwrap();
        assert_eq!(schema_snapshot(database.write()).await.unwrap(), adopted);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn adopt_doi_pk_va_giu_fk() {
    TestDatabase::run(async |test| {
        let (database, parent, _) = seeded(test).await;
        full().up(&database).await.unwrap();
        // Khoá chính mới: cùng `key` ở hai khách được; unique theo khách.
        let other = Uuid::now_v7();
        run(
            &database,
            "INSERT INTO demo_kv (tenant_id, key, value) VALUES (?, ?, ?)",
            vec![other.into(), "k".into(), "w".into()],
        )
        .await;
        run(
            &database,
            "INSERT INTO demo_parent (tenant_id, id, name) VALUES (?, ?, ?)",
            vec![other.into(), Uuid::now_v7().into(), "Ada".into()],
        )
        .await;
        assert_eq!(count(&database, "SELECT COUNT(*) FROM demo_kv").await, 2);
        // Khoá ngoại còn: xoá cha thì con đi theo.
        run(&database, "DELETE FROM demo_parent WHERE id = ?", vec![parent.into()]).await;
        assert_eq!(count(&database, "SELECT COUNT(*) FROM demo_child").await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn adopt_down_tu_choi_khi_con_khach_khac() {
    TestDatabase::run(async |test| {
        let (database, _, before) = seeded(test).await;
        full().up(&database).await.unwrap();
        let other = Uuid::now_v7();
        run(
            &database,
            "INSERT INTO demo_kv (tenant_id, key, value) VALUES (?, ?, ?)",
            vec![other.into(), "k".into(), "w".into()],
        )
        .await;
        let refused = full().down(&database, "demo", 1).await.unwrap_err();
        assert!(refused.to_string().contains("other than the default"), "{refused}");
        // Từ chối không đổi gì: dữ liệu còn nguyên.
        assert_eq!(count(&database, "SELECT COUNT(*) FROM demo_kv").await, 2);
        run(&database, "DELETE FROM demo_kv WHERE tenant_id = ?", vec![other.into()]).await;
        full().down(&database, "demo", 1).await.unwrap();
        assert_eq!(
            demo_tables(&schema_snapshot(database.write()).await.unwrap()),
            demo_tables(&before)
        );
        assert_eq!(count(&database, "SELECT COUNT(*) FROM demo_kv").await, 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn adopt_tu_choi_khi_quen_index_cu() {
    TestDatabase::run(async |test| {
        let database = connect(test).await;
        migrator(vec![Box::new(CreateDemo)]).up(&database).await.unwrap();
        let forgot = adopt(
            "m0002_tenancy",
            vec![
                TenantTable::new("demo_parent")
                    .unique("demo_parent_name_unique", ["tenant_id", "name"]),
                TenantTable::new("demo_child")
                    .unique("demo_child_code_unique", ["tenant_id", "parent_id", "code"]),
                TenantTable::new("demo_kv").primary_key(["tenant_id", "key"]),
            ],
        );
        let error = migrator(vec![Box::new(CreateDemo), forgot]).up(&database).await.unwrap_err();
        assert!(error.to_string().contains("demo_child_created_idx"), "{error}");
        let no_key = adopt(
            "m0002_tenancy",
            vec![
                TenantTable::new("demo_parent")
                    .unique("demo_parent_name_unique", ["tenant_id", "name"]),
                TenantTable::new("demo_child")
                    .unique("demo_child_code_unique", ["tenant_id", "parent_id", "code"])
                    .drop_index("demo_child_created_idx"),
                TenantTable::new("demo_kv"),
            ],
        );
        let error = migrator(vec![Box::new(CreateDemo), no_key]).up(&database).await.unwrap_err();
        assert!(error.to_string().contains("primary key"), "{error}");
        // Con nâng mà cha không cùng đợt: từ chối (xoá cha cũ sẽ xoá dây chuyền dữ liệu của con).
        let split = adopt(
            "m0002_tenancy",
            vec![
                TenantTable::new("demo_parent")
                    .unique("demo_parent_name_unique", ["tenant_id", "name"]),
            ],
        );
        let error = migrator(vec![Box::new(CreateDemo), split]).up(&database).await.unwrap_err();
        assert!(error.to_string().contains("demo_child"), "{error}");
        // Hỏng giữa chừng không đổi gì: bảng vẫn chưa có `tenant_id`.
        let snapshot = schema_snapshot(database.write()).await.unwrap();
        assert!(
            snapshot.tables.iter().all(|table| table.columns.iter().all(|c| c.name != "tenant_id"))
        );
    })
    .await
    .unwrap();
}
