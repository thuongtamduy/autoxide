//! Database mẫu đã migrate (D-20261120-test-db-template): `TestApp` chép mẫu khi database của bài
//! test còn nguyên, kết quả như migrate từ đầu, dữ liệu không lọt giữa các bài test; database đã có
//! người dùng hay bài test chọn `empty_database()` thì migrate như cũ — trên cả hai database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::module::Module;
use autoxide_testkit::{TestApp, TestDatabase, template_enabled};
use sea_orm::{ConnectionTrait, Statement};

struct M0001Items;

impl MigrationName for M0001Items {
    fn name(&self) -> &str {
        "m0001_items"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for M0001Items {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("tplcheck_items")
                    .col(ColumnDef::new("id").big_integer().not_null().primary_key())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("tplcheck_items").to_owned()).await
    }
}

struct TplCheck;

impl Module for TplCheck {
    fn name(&self) -> &'static str {
        "tplcheck"
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        vec![Box::new(M0001Items)]
    }
}

async fn count(app: &TestApp) -> i64 {
    let row = app
        .database()
        .read()
        .query_one_raw(Statement::from_string(
            app.database().read().get_database_backend(),
            "SELECT count(*) FROM tplcheck_items",
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get_by_index(0).unwrap()
}

async fn insert(app: &TestApp, id: i64) {
    let sql = format!("INSERT INTO tplcheck_items (id) VALUES ({id})");
    app.database().write().execute_unprepared(&sql).await.unwrap();
}

#[tokio::test]
async fn database_con_nguyen_thi_chep_mau_va_khong_lot_du_lieu() {
    for round in 0..2 {
        TestDatabase::run(async |test| {
            let app = TestApp::builder(test).module(TplCheck).build().await.unwrap();
            // `TEST_DATABASE_TEMPLATE=off` tắt mẫu: khi đó migrate từ đầu, kết quả như nhau.
            assert_eq!(app.from_template(), template_enabled(), "round {round}");
            // Bảng có, trống: dữ liệu của vòng trước không nằm trong mẫu.
            assert_eq!(count(&app).await, 0, "round {round}");
            insert(&app, 1).await;
            assert_eq!(count(&app).await, 1);
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn empty_database_thi_migrate_tu_dau() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(TplCheck).empty_database().build().await.unwrap();
        assert!(!app.from_template());
        assert_eq!(count(&app).await, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn database_da_co_du_lieu_thi_khong_chep_de() {
    TestDatabase::run(async |test| {
        let connection = test.connect().await.unwrap();
        connection
            .execute_unprepared("CREATE TABLE outside_owned (id BIGINT PRIMARY KEY)")
            .await
            .unwrap();
        connection.execute_unprepared("INSERT INTO outside_owned (id) VALUES (7)").await.unwrap();
        connection.close().await.unwrap();

        let app = TestApp::builder(test).module(TplCheck).build().await.unwrap();
        assert!(!app.from_template());
        assert_eq!(count(&app).await, 0);
        // Thứ bài test đặt sẵn vẫn còn.
        let row = app
            .database()
            .read()
            .query_one_raw(Statement::from_string(
                app.database().read().get_database_backend(),
                "SELECT count(*) FROM outside_owned",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get_by_index::<i64>(0).unwrap(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhieu_bai_test_cung_luc_dung_chung_mot_mau() {
    // Tám bài test song song cùng cần mẫu: một bên dựng, các bên khác chờ rồi chép.
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            tokio::spawn(async {
                TestDatabase::run(async |test| {
                    let app = TestApp::builder(test).module(TplCheck).build().await.unwrap();
                    insert(&app, 1).await;
                    (app.from_template(), count(&app).await)
                })
                .await
                .unwrap()
            })
        })
        .collect();
    for task in tasks {
        assert_eq!(task.await.unwrap(), (template_enabled(), 1));
    }
}
