//! Bộ kiểm "module sở hữu bảng của mình" phải bắt được vi phạm trên cả SQLite và
//! Postgres (gate chạy file này trên cả hai).

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::{TestDatabase, schema_ownership_violations};
use sea_orm::sea_query::{ColumnDef, ForeignKey, Table, TableCreateStatement};
use sea_orm::{ConnectionTrait, DatabaseConnection};

/// Bảng chỉ có khoá chính `id`.
fn table(name: &'static str) -> TableCreateStatement {
    Table::create()
        .table(name)
        .col(ColumnDef::new("id").integer().not_null().primary_key())
        .to_owned()
}

/// Bảng có thêm cột `parent_id` là foreign key trỏ tới `parent(id)`.
fn child_table(name: &'static str, parent: &'static str) -> TableCreateStatement {
    table(name)
        .col(ColumnDef::new("parent_id").integer().not_null())
        .foreign_key(ForeignKey::create().from(name, "parent_id").to(parent, "id"))
        .to_owned()
}

async fn create(connection: &DatabaseConnection, tables: &[TableCreateStatement]) {
    for table in tables {
        connection.execute(table).await.unwrap();
    }
}

#[tokio::test]
async fn database_trong_va_schema_dung_luat_thi_khong_vi_pham() {
    TestDatabase::run(async |database| {
        let connection = database.connect().await.unwrap();
        assert!(schema_ownership_violations(&connection, &[]).await.unwrap().is_empty());

        create(
            &connection,
            &[
                table("auth_user"),
                // Foreign key TRONG một module thì được.
                child_table("auth_session", "auth_user"),
                table("notes_note"),
                table("notes_migrations"),
            ],
        )
        .await;

        let violations =
            schema_ownership_violations(&connection, &["auth", "notes"]).await.unwrap();
        assert_eq!(violations, Vec::<String>::new());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn bang_khong_mang_tien_to_module_bi_bao() {
    TestDatabase::run(async |database| {
        let connection = database.connect().await.unwrap();
        create(&connection, &[table("auth_user"), table("orphan"), table("billing_invoice")]).await;

        let violations = schema_ownership_violations(&connection, &["auth"]).await.unwrap();

        assert_eq!(
            violations,
            [
                "table `billing_invoice` belongs to no registered module: name it `<module>_…`",
                "table `orphan` belongs to no registered module: name it `<module>_…`",
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn foreign_key_xuyen_module_bi_bao() {
    TestDatabase::run(async |database| {
        let connection = database.connect().await.unwrap();
        create(
            &connection,
            &[
                table("auth_user"),
                // Module notes trỏ foreign key vào bảng người dùng của module auth:
                // đúng thứ luật cấm, kể cả với bảng người dùng (D-025).
                child_table("notes_note", "auth_user"),
            ],
        )
        .await;

        let violations =
            schema_ownership_violations(&connection, &["auth", "notes"]).await.unwrap();

        assert_eq!(
            violations,
            ["foreign key from `notes_note` (module `notes`) to `auth_user` (module `auth`): \
              modules must not share foreign keys — keep only the ID"]
        );
    })
    .await
    .unwrap();
}
