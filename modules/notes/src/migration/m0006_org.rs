//! Ghi chú thuộc một nút của cây tổ chức (bước 34 mục 2): `org_id`, ghi chú cũ ở nút gốc
//! (= ID của khách). Cột NULL được ở database, điền ngay ở đây và service luôn ghi (D-304 mục 6).

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, Statement};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_org"
    }
}

#[autoxide_kernel::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("notes_note")
                    .add_column(schema::uuid_null("org_id"))
                    .to_owned(),
            )
            .await?;
        let connection = manager.get_connection();
        let backend = connection.get_database_backend();
        connection
            .execute_raw(Statement::from_string(
                backend,
                "UPDATE notes_note SET org_id = tenant_id".to_owned(),
            ))
            .await?;
        // Danh sách theo nhánh: `org_id IN (nhánh)` rồi sắp theo `id`.
        manager
            .create_index(
                Index::create()
                    .name("notes_note_org_idx")
                    .table("notes_note")
                    .col("tenant_id")
                    .col("org_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name("notes_note_org_idx").table("notes_note").to_owned())
            .await?;
        manager
            .alter_table(Table::alter().table("notes_note").drop_column("org_id").to_owned())
            .await
    }
}
