//! Cột `custom_fields` của `content_published` (bước 51 P19): trường tự định nghĩa chép từ mục khi
//! đăng, JSON trong `text`. Bản công khai có sẵn: `NULL` (đăng lại thì có).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0020_published_custom_fields"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("content_published")
                    .add_column(schema::text_null("custom_fields"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("content_published").drop_column("custom_fields").to_owned(),
            )
            .await
    }
}
