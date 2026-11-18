//! Cột `custom_fields` của `content_entries` (bước 51 P19, D-235): giá trị trường tự định nghĩa của
//! khách, JSON trong `text` (như nhau trên SQLite và Postgres). Dòng có sẵn: `NULL` (không có giá trị).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0018_custom_fields"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("content_entries")
                    .add_column(schema::text_null("custom_fields"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("content_entries").drop_column("custom_fields").to_owned(),
            )
            .await
    }
}
