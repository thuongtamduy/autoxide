//! m0010: từ đồng nghĩa theo khách cho tìm và gợi ý tìm (D-201); index cho sắp theo điểm đánh giá
//! và đếm theo người bán trên storefront.

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateSearch;

impl MigrationName for CreateSearch {
    fn name(&self) -> &str {
        "m0010_search"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateSearch {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Một nhóm từ đồng nghĩa một dòng: `terms` là JSON danh sách từ đã bỏ dấu, chữ thường.
        let mut synonyms = table("catalog_synonyms");
        synonyms
            .col(schema::integer("position"))
            .col(schema::text("terms"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(synonyms).await?;
        index(
            manager,
            "catalog_synonyms",
            "catalog_synonyms_position_idx",
            &["tenant_id", "position"],
            false,
        )
        .await?;
        let t = "catalog_storefront_entries";
        index(
            manager,
            t,
            "catalog_storefront_rating_idx",
            &["tenant_id", "org_id", "visible", "rating_x100", "product_id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_storefront_owner_idx",
            &["tenant_id", "org_id", "visible", "owner_org_id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in ["catalog_storefront_owner_idx", "catalog_storefront_rating_idx"] {
            manager
                .drop_index(
                    Index::drop()
                        .name(name)
                        .table(Alias::new("catalog_storefront_entries"))
                        .to_owned(),
                )
                .await?;
        }
        manager.drop_table(Table::drop().table(Alias::new("catalog_synonyms")).to_owned()).await
    }
}
