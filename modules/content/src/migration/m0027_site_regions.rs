//! Đầu trang và chân trang của website (bước 67, D-234, D-20261122-public-site): mỗi khu là một bố cục
//! khối theo (website, khu, ngôn ngữ, ô chủ đề). `variant_slot` là `base` hay ID bản chủ đề dạng chữ —
//! không dùng NULL vì unique coi NULL là khác nhau trên cả hai database. Sửa vào `draft_layout`, đăng
//! mới thành `layout` công khai (đầu trang hiện trên mọi trang nên không đăng thẳng).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0027_site_regions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_site_regions")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    // `header` hay `footer`.
                    .col(schema::string_len("region", 8))
                    .col(schema::string_len("locale", 35))
                    .col(schema::string_len("variant_slot", 36))
                    .col(schema::text_null("layout"))
                    .col(schema::text_null("draft_layout"))
                    .col(schema::timestamp_with_time_zone_null("published_at"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_site_regions_unique")
                    .table("content_site_regions")
                    .col("tenant_id")
                    .col("org_id")
                    .col("region")
                    .col("locale")
                    .col("variant_slot")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_site_regions").to_owned()).await
    }
}
