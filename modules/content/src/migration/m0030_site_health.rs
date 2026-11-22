//! Kiểm tra sức khoẻ website (bước 67, D-20261122-site-health): kết quả lần kiểm gần nhất của mỗi website
//! (một dòng mỗi website), để hộp việc đếm và giao diện hiện.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0030_site_health"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_site_health")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    // JSON: danh sách phát hiện (loại, mục, đường dẫn, chi tiết, cách sửa).
                    .col(schema::text("findings"))
                    .col(schema::integer("findings_count"))
                    .col(schema::uuid_null("checked_by"))
                    .col(schema::timestamp_with_time_zone("checked_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_site_health_org_unique")
                    .table("content_site_health")
                    .col("tenant_id")
                    .col("org_id")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_site_health").to_owned()).await
    }
}
