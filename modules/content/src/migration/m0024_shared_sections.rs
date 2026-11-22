//! Khu dùng chung nhiều trang (bước 67, D-20261122-shared-sections): một bố cục khối đặt vào nhiều
//! trang qua khối `content.shared_section {id}`; sửa khu là mọi trang đổi theo lúc đọc công khai.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0024_shared_sections"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_shared_sections")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("name", 120))
                    // Bố cục khối JSON (`{"type": "layout", …}`), như thân `layout` của trang.
                    .col(schema::text("layout"))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Danh sách theo khách sắp theo `id` (D-302).
        manager
            .create_index(
                Index::create()
                    .name("content_shared_sections_list_idx")
                    .table("content_shared_sections")
                    .col("tenant_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_shared_sections").to_owned()).await
    }
}
