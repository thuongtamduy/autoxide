//! Lùi database về các migration của một bản trước (D-20261126-upgrade-rollback): kế hoạch (chạy thử, không đổi gì)
//! nêu đúng bước và dữ liệu sẽ mất; lùi thật gỡ đúng các bước đó, ngược thứ tự; từ chối khi phải bỏ qua một migration
//! giữa chừng hay gặp migration khai không lùi được. Cả SQLite và Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{DryRunLimits, LostData, Migrator, ModuleMigrations};
use autoxide_testkit::TestDatabase;
use sea_orm::ConnectionTrait;

struct CreateTable(&'static str, &'static str);

impl MigrationName for CreateTable {
    fn name(&self) -> &str {
        self.0
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(Table::create().table(self.1).col(schema::pk_uuid("id")).to_owned())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(self.1).to_owned()).await
    }
}

struct AddColumn;

impl MigrationName for AddColumn {
    fn name(&self) -> &str {
        "m0002_add_title"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddColumn {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("shop_item")
                    .add_column(schema::string_null("title"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.alter_table(Table::alter().table("shop_item").drop_column("title").to_owned()).await
    }
}

/// `down` xoá mọi dòng hàng (bảng giữ lại): phải tính là mất dữ liệu.
struct Purge;

impl MigrationName for Purge {
    fn name(&self) -> &str {
        "m0004_purge_on_down"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Purge {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.get_connection().execute_unprepared("DELETE FROM shop_item").await.map(drop)
    }
}

/// Bước thu hẹp: khai không lùi được.
struct Contract;

impl MigrationName for Contract {
    fn name(&self) -> &str {
        "m0005_contract"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Contract {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    fn irreversible(&self) -> bool {
        true
    }
}

fn migrator(with_contract: bool) -> Migrator {
    let mut shop: Vec<Box<dyn MigrationTrait>> = vec![
        Box::new(CreateTable("m0001_create_item", "shop_item")),
        Box::new(AddColumn),
        Box::new(CreateTable("m0003_create_order", "shop_order")),
        Box::new(Purge),
    ];
    if with_contract {
        shop.push(Box::new(Contract));
    }
    Migrator::new(vec![
        ModuleMigrations::new("shop", shop),
        ModuleMigrations::new("tags", vec![Box::new(CreateTable("m0001_create_tag", "tags_tag"))]),
    ])
    .unwrap()
}

async fn connect(database: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    Database::connect(&reader.finish(config).unwrap().0).await.unwrap()
}

async fn run(database: &Database, sql: &str) {
    let backend = database.write().get_database_backend();
    database.write().execute_raw(sea_orm::Statement::from_string(backend, sql)).await.unwrap();
}

/// Số thư mục bản sao của chạy thử còn sót cạnh file SQLite (Postgres: 0).
fn copies_left(test: &TestDatabase) -> usize {
    let Some(rest) = test.url().strip_prefix("sqlite://") else {
        return 0;
    };
    let file = std::path::Path::new(rest.split('?').next().unwrap());
    std::fs::read_dir(file.parent().unwrap())
        .unwrap()
        .filter(|entry| {
            entry.as_ref().unwrap().file_name().to_string_lossy().starts_with(".autoxide-rollback-")
        })
        .count()
}

/// Bản đích chỉ có `shop/m0001` và `tags/m0001`.
fn target(module: &str, name: &str) -> bool {
    matches!((module, name), ("shop", "m0001_create_item") | ("tags", "m0001_create_tag"))
}

#[tokio::test]
async fn ke_hoach_neu_buoc_va_du_lieu_mat_roi_lui_that() {
    TestDatabase::run(async |test| {
        let db = connect(test).await;
        let migrator = migrator(false);
        migrator.up(&db).await.unwrap();
        // Hai dòng hàng, một có `title`; một đơn hàng.
        run(&db, "INSERT INTO shop_item (id, title) VALUES ('00000000-0000-7000-8000-000000000001', 'a')").await;
        run(&db, "INSERT INTO shop_item (id) VALUES ('00000000-0000-7000-8000-000000000002')").await;
        run(&db, "INSERT INTO shop_order (id) VALUES ('00000000-0000-7000-8000-000000000003')").await;

        let plan = migrator.rollback_plan(&db, &target, DryRunLimits::default()).await.unwrap();
        let steps: Vec<String> = plan.steps.iter().map(|step| format!("{}/{}", step.module, step.name)).collect();
        assert_eq!(steps, ["shop/m0004_purge_on_down", "shop/m0003_create_order", "shop/m0002_add_title"]);
        let mut loss = plan.data_loss.clone();
        loss.sort_by(|a, b| a.table.cmp(&b.table));
        let shown: Vec<(&str, Option<&str>, u64, bool)> = loss
            .iter()
            .map(|lost: &LostData| (lost.table.as_str(), lost.column.as_deref(), lost.rows, lost.dropped))
            .collect();
        // Cột `title`: một giá trị khác NULL; `down` của m0004 xoá hai dòng hàng (bảng giữ lại); bảng đơn bị bỏ.
        assert_eq!(
            shown,
            [("shop_item", Some("title"), 1, false), ("shop_item", None, 2, false), ("shop_order", None, 1, true)]
        );
        assert_eq!(plan.lost_rows(), 4);
        // SQLite: bản sao của chạy thử nằm trong thư mục riêng cạnh file database và đã bị xoá.
        assert_eq!(copies_left(test), 0);
        // Chạy thử không đổi gì.
        assert!(migrator.pending(&db).await.unwrap().is_empty());
        assert!(SchemaManager::new(db.read()).has_table("shop_order").await.unwrap());

        let reverted = migrator.rollback(&db, &target).await.unwrap();
        assert_eq!(reverted.len(), 3);
        let pending: Vec<String> = migrator.pending(&db).await.unwrap().into_iter().map(|state| state.name).collect();
        assert_eq!(pending, ["m0002_add_title", "m0003_create_order", "m0004_purge_on_down"]);
        assert!(!SchemaManager::new(db.read()).has_table("shop_order").await.unwrap());
        // Lùi rồi tiến lại được.
        migrator.up(&db).await.unwrap();
        assert!(migrator.rollback_plan(&db, &|_, _| true, DryRunLimits::default()).await.unwrap().steps.is_empty());
        db.close().await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tu_choi_khi_bo_giua_chung_hay_khong_lui_duoc() {
    TestDatabase::run(async |test| {
        let db = connect(test).await;
        let migrator = migrator(true);
        migrator.up(&db).await.unwrap();

        let error = migrator
            .rollback_plan(&db, &target, DryRunLimits::default())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("`m0005_contract` cannot be reverted"), "{error}");
        assert!(error.contains("restore the backup"), "{error}");

        // Giữ m0003 mà bỏ m0002 chạy trước nó: không đạt được bằng cách gỡ các migration mới nhất.
        let gap = |module: &str, name: &str| module != "shop" || name != "m0002_add_title";
        let error = migrator
            .rollback_plan(&db, &gap, DryRunLimits::default())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("keeps `m0005_contract` but not `m0002_add_title`"), "{error}");
        db.close().await.unwrap();
    })
    .await
    .unwrap();
}
