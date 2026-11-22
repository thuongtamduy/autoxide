//! Bộ giao diện của website (bước 67, D-20261122-theme-tokens): `content_themes` giữ phần GHI ĐÈ
//! token của một nút cây tổ chức (gốc = cả khách). Một dòng mỗi (khách, nút). Token là JSON trong
//! `text` (chạy như nhau trên hai database).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0023_themes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_themes")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::text("tokens"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_themes_org_unique")
                    .table("content_themes")
                    .col("tenant_id")
                    .col("org_id")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_themes").to_owned()).await
    }
}
