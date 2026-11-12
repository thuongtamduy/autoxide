//! Nhãn (`media_tag`, `media_item_tag`) và ảnh thu nhỏ (`media_variant`) — D-093, D-094.

use autoxide_kernel::migration::prelude::*;
use sea_orm::DbBackend;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0004_create_tags_and_variants"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let backend = manager.get_database_backend();
        // `name_key` so sánh theo byte trên cả hai database, như ở m0001.
        let mut name_key = schema::text("name_key");
        if backend == DbBackend::Postgres {
            name_key.extra("COLLATE \"C\"");
        }
        manager
            .create_table(
                Table::create()
                    .table("media_tag")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("name", 50))
                    .col(name_key)
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Không có hai nhãn cùng tên (không phân biệt hoa thường); cũng để tìm và sắp theo tên.
        manager
            .create_index(
                Index::create()
                    .name("media_tag_name_unique")
                    .table("media_tag")
                    .col("name_key")
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("media_item_tag")
                    .col(schema::uuid("item_id"))
                    .col(schema::uuid("tag_id"))
                    .primary_key(Index::create().col("item_id").col("tag_id"))
                    .to_owned(),
            )
            .await?;
        // Lọc item theo nhãn, đếm item của nhãn.
        manager
            .create_index(
                Index::create()
                    .name("media_item_tag_tag_idx")
                    .table("media_item_tag")
                    .col("tag_id")
                    .col("item_id")
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table("media_variant")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("item_id"))
                    // Phiên bản nội dung gốc mà ảnh thu nhỏ này làm từ đó.
                    .col(schema::uuid("version_id"))
                    .col(schema::integer("width"))
                    .col(schema::integer("height"))
                    .col(schema::string_len("content_type", 255))
                    .col(schema::big_integer("size"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("media_variant_unique")
                    .table("media_variant")
                    .col("item_id")
                    .col("version_id")
                    .col("width")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["media_variant", "media_item_tag", "media_tag"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
