//! Migration theo module phải chạy giống nhau trên SQLite và Postgres. Gate chạy
//! file này trên cả hai.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, DatabaseConfig};
use autoxide_kernel::database::Database;
use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::migration::{MigrationState, Migrator, ModuleMigrations};
use autoxide_testkit::TestDatabase;

/// Migration tạo một bảng chỉ có cột `id`; `down` xoá bảng đó.
struct CreateTable {
    name: &'static str,
    table: &'static str,
}

impl MigrationName for CreateTable {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateTable {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(Table::create().table(self.table).col(schema::pk_uuid("id")).to_owned())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(self.table).to_owned()).await
    }
}

/// Migration tạo được một bảng rồi hỏng.
struct FailsHalfway;

impl MigrationName for FailsHalfway {
    fn name(&self) -> &str {
        "m0001_fails_halfway"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for FailsHalfway {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create().table("broken_partial").col(schema::pk_uuid("id")).to_owned(),
            )
            .await?;
        Err(DbErr::Migration("deliberate failure".to_owned()))
    }
}

fn notes() -> Vec<Box<dyn MigrationTrait>> {
    vec![
        Box::new(CreateTable { name: "m0001_create_note", table: "notes_note" }),
        Box::new(CreateTable { name: "m0002_create_draft", table: "notes_draft" }),
    ]
}

fn notes_first_only() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(CreateTable { name: "m0001_create_note", table: "notes_note" })]
}

fn tags() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(CreateTable { name: "m0001_create_tag", table: "tags_tag" })]
}

fn broken() -> Vec<Box<dyn MigrationTrait>> {
    vec![Box::new(FailsHalfway)]
}

fn migrator() -> Migrator {
    Migrator::new(vec![
        ModuleMigrations::new("notes", notes()),
        ModuleMigrations::new("tags", tags()),
    ])
    .unwrap()
}

async fn connect(database: &TestDatabase) -> Database {
    let source = ConfigSource::from_pairs([("DATABASE_URL", database.url())]);
    let mut reader = ConfigReader::new(&source);
    let config = DatabaseConfig::read(&mut reader);
    Database::connect(&reader.finish(config).unwrap().0).await.unwrap()
}

async fn has_table(database: &Database, table: &str) -> bool {
    SchemaManager::new(database.read()).has_table(table).await.unwrap()
}

/// Rút gọn danh sách trạng thái thành `["module/tên", …]` để so sánh cho gọn.
fn names(states: &[MigrationState]) -> Vec<String> {
    states.iter().map(|state| format!("{}/{}", state.module, state.name)).collect()
}

const ALL: [&str; 3] =
    ["notes/m0001_create_note", "notes/m0002_create_draft", "tags/m0001_create_tag"];

#[tokio::test]
async fn status_tren_database_trong_khong_tao_gi() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;

        let status = migrator().status(&db).await.unwrap();

        assert_eq!(names(&status), ALL);
        assert!(status.iter().all(|state| !state.applied));
        // Xem trạng thái là việc chỉ đọc: không được tạo cả bảng lịch sử.
        assert!(!has_table(&db, "notes_migrations").await);
        assert!(!has_table(&db, "tags_migrations").await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn up_ap_dung_moi_migration_con_thieu_cua_moi_module() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let migrator = migrator();

        let applied = migrator.up(&db).await.unwrap();

        assert_eq!(names(&applied), ALL);
        assert!(migrator.status(&db).await.unwrap().iter().all(|state| state.applied));
        assert!(migrator.pending(&db).await.unwrap().is_empty());
        for table in
            ["notes_note", "notes_draft", "tags_tag", "notes_migrations", "tags_migrations"]
        {
            assert!(has_table(&db, table).await, "{table}");
        }

        // Chạy lại không làm gì và không lỗi.
        assert!(migrator.up(&db).await.unwrap().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn down_chi_go_migration_cua_mot_module() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let migrator = migrator();
        migrator.up(&db).await.unwrap();

        let reverted = migrator.down(&db, "notes", 1).await.unwrap();

        assert_eq!(names(&reverted), ["notes/m0002_create_draft"]);
        assert!(!has_table(&db, "notes_draft").await);
        assert!(has_table(&db, "notes_note").await);
        assert!(has_table(&db, "tags_tag").await);
        assert_eq!(names(&migrator.pending(&db).await.unwrap()), ["notes/m0002_create_draft"]);

        // Xin gỡ nhiều hơn số đang có thì gỡ hết phần còn lại, mới nhất trước.
        let reverted = migrator.down(&db, "notes", 5).await.unwrap();
        assert_eq!(names(&reverted), ["notes/m0001_create_note"]);
        assert!(!has_table(&db, "notes_note").await);
        assert!(migrator.down(&db, "notes", 1).await.unwrap().is_empty());
        // Module khác không bị đụng.
        assert!(has_table(&db, "tags_tag").await);

        // Lên lại được sau khi đã xuống.
        let applied = migrator.up(&db).await.unwrap();
        assert_eq!(names(&applied), ["notes/m0001_create_note", "notes/m0002_create_draft"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_hong_giua_chung_khong_de_lai_gi() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        let migrator = Migrator::new(vec![
            ModuleMigrations::new("tags", tags()),
            ModuleMigrations::new("broken", broken()),
        ])
        .unwrap();

        let error = migrator.up(&db).await.unwrap_err().to_string();

        assert!(error.contains("module `broken`: cannot apply migrations"), "{error}");
        assert!(error.contains("deliberate failure"), "{error}");
        // Bảng tạo dở trong migration hỏng đã bị rollback — trên cả hai database.
        assert!(!has_table(&db, "broken_partial").await);
        // Migration hỏng vẫn ở trạng thái chưa áp dụng; module đứng trước đã xong.
        assert_eq!(names(&migrator.pending(&db).await.unwrap()), ["broken/m0001_fails_halfway"]);
        assert!(has_table(&db, "tags_tag").await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn down_module_khong_ton_tai_thi_bao_ro() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;

        let error = migrator().down(&db, "nope", 1).await.unwrap_err().to_string();

        assert_eq!(error, "migration: unknown module `nope`; registered modules: notes, tags");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn migration_da_ap_dung_ma_mat_khoi_code_la_loi() {
    TestDatabase::run(async |database| {
        let db = connect(database).await;
        migrator().up(&db).await.unwrap();

        // Bản code "cũ hơn database": thiếu migration thứ hai đã được áp dụng.
        let older =
            Migrator::new(vec![ModuleMigrations::new("notes", notes_first_only())]).unwrap();
        let error = older.status(&db).await.unwrap_err().to_string();

        assert!(error.contains("module `notes`"), "{error}");
        assert!(error.contains("m0002_create_draft"), "{error}");
    })
    .await
    .unwrap();
}
