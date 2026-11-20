//! m0015: bản nháp AI cho mô tả và SEO của sản phẩm (`catalog_suggestions`, D-109). Bản nháp
//! không bao giờ tự áp: người duyệt áp (`applied`) hay bỏ (`discarded`).

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateSuggestions;

impl MigrationName for CreateSuggestions {
    fn name(&self) -> &str {
        "m0015_suggestions"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSuggestions {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut suggestions = table("catalog_suggestions");
        suggestions
            .col(schema::uuid("product_id"))
            .col(schema::string_len("locale", 16))
            // JSON: `short_description`, `description`, `meta_title`, `meta_description`.
            .col(schema::text("fields"))
            .col(schema::string_len("status", 16))
            .col(schema::string_len("model", 120))
            .col(schema::uuid_null("created_by"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone_null("decided_at"));
        manager.create_table(suggestions).await?;
        index(
            manager,
            "catalog_suggestions",
            "catalog_suggestions_product_idx",
            &["tenant_id", "product_id", "created_at"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("catalog_suggestions")).to_owned()).await
    }
}
