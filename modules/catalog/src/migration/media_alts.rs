//! m0017: `alt` của ảnh sản phẩm theo ngôn ngữ (D-202). `catalog_product_media.alt` giữ nghĩa bản
//! ngôn ngữ mặc định; bản ghi đè theo ngôn ngữ nằm ở đây.

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateMediaAlts;

impl MigrationName for CreateMediaAlts {
    fn name(&self) -> &str {
        "m0017_media_alts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateMediaAlts {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut alts = table("catalog_product_media_alts");
        alts.col(schema::uuid("product_id"))
            .col(schema::uuid("media_id"))
            .col(schema::string_len("locale", 16))
            .col(schema::string_len("alt", 500))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(alts).await?;
        index(
            manager,
            "catalog_product_media_alts",
            "catalog_product_media_alts_uq",
            &["tenant_id", "product_id", "media_id", "locale"],
            true,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new("catalog_product_media_alts")).to_owned())
            .await
    }
}
