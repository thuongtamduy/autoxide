//! m0020: lần tạo sản phẩm nháp từ ảnh bằng AI (`catalog_image_drafts`, D-229). Trạng thái một chiều:
//! `queued` → `running` → `succeeded` (có `product_id`) / `failed` (có `error_code`).

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateImageDrafts;

impl MigrationName for CreateImageDrafts {
    fn name(&self) -> &str {
        "m0020_image_drafts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateImageDrafts {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut drafts = table("catalog_image_drafts");
        drafts
            .col(schema::uuid("org_id"))
            // JSON: ID ảnh trong `media`, theo thứ tự gửi.
            .col(schema::text("media_ids"))
            .col(schema::string_len("locale", 35))
            .col(schema::uuid_null("category_id"))
            .col(schema::string_len("tax_category", 64))
            .col(schema::string_len("status", 16))
            .col(schema::uuid_null("product_id"))
            .col(schema::string_len_null("error_code", 120))
            .col(schema::string_len_null("model", 120))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone_null("finished_at"));
        manager.create_table(drafts).await?;
        index(
            manager,
            "catalog_image_drafts",
            "catalog_image_drafts_created_idx",
            &["tenant_id", "created_by", "id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("catalog_image_drafts")).to_owned()).await
    }
}
