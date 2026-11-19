//! m0011: bộ sưu tập sản phẩm (bước 55 mục 3): chọn tay (có thứ tự) hay theo quy tắc, có trang
//! công khai theo slug và ngôn ngữ.
//!
//! - `catalog_collections`: nút sở hữu, loại, quy tắc (JSON đã kiểm), kiểu sắp, ảnh, trạng thái,
//!   khung giờ.
//! - `catalog_collection_texts`: tiêu đề, slug, mô tả, SEO theo ngôn ngữ; slug không trùng trong
//!   một ngôn ngữ của khách.
//! - `catalog_collection_items`: sản phẩm chọn tay và thứ tự.

use autoxide_kernel::migration::prelude::*;

use super::{index, table};

pub(super) struct CreateCollections;

impl MigrationName for CreateCollections {
    fn name(&self) -> &str {
        "m0011_collections"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCollections {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut collections = table("catalog_collections");
        collections
            .col(schema::uuid("org_id"))
            .col(schema::string_len("kind", 16))
            .col(schema::text("rules"))
            .col(schema::string_len("sort", 16))
            .col(schema::uuid_null("media_id"))
            .col(schema::string_len("status", 16))
            .col(schema::timestamp_with_time_zone_null("starts_at"))
            .col(schema::timestamp_with_time_zone_null("ends_at"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(collections).await?;
        let t = "catalog_collections";
        index(manager, t, "catalog_collections_status_idx", &["tenant_id", "status", "id"], false)
            .await?;
        index(manager, t, "catalog_collections_org_idx", &["tenant_id", "org_id", "id"], false)
            .await?;

        let mut texts = table("catalog_collection_texts");
        texts
            .col(schema::uuid("collection_id"))
            .col(schema::string_len("locale", 35))
            .col(schema::string_len("title", 200))
            .col(schema::string_len("slug", 200))
            .col(schema::text("description"))
            .col(schema::string_len("meta_title", 200))
            .col(schema::string_len("meta_description", 500));
        manager.create_table(texts).await?;
        let t = "catalog_collection_texts";
        index(
            manager,
            t,
            "catalog_collection_texts_slug_unique",
            &["tenant_id", "locale", "slug"],
            true,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_collection_texts_locale_unique",
            &["tenant_id", "collection_id", "locale"],
            true,
        )
        .await?;

        let mut items = table("catalog_collection_items");
        items
            .col(schema::uuid("collection_id"))
            .col(schema::uuid("product_id"))
            .col(schema::integer("position"));
        manager.create_table(items).await?;
        let t = "catalog_collection_items";
        index(
            manager,
            t,
            "catalog_collection_items_unique",
            &["tenant_id", "collection_id", "product_id"],
            true,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_collection_items_position_idx",
            &["tenant_id", "collection_id", "position", "product_id"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "catalog_collection_items_product_idx",
            &["tenant_id", "product_id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for name in ["catalog_collection_items", "catalog_collection_texts", "catalog_collections"]
        {
            manager.drop_table(Table::drop().table(Alias::new(name)).to_owned()).await?;
        }
        Ok(())
    }
}
