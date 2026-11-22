//! Bản chủ đề theo lịch (bước 67, D-234, D-20261122-theme-variants): ghi đè một phần theme gốc của một
//! website theo lịch (một lần, hằng ngày, hằng tuần), có ưu tiên, nháp / bật / tạm dừng.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0026_theme_variants"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_theme_variants")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("name", 120))
                    // Phần ghi đè token (JSON trong `text`), cùng bộ kiểm với theme gốc.
                    .col(schema::text("overrides"))
                    .col(schema::string_len_null("effect", 16))
                    .col(schema::string_len_null("effect_intensity", 8))
                    .col(schema::text("schedule"))
                    // Lịch `once`: khoảng UTC, để lọc nhanh; lịch lặp: NULL.
                    .col(schema::timestamp_with_time_zone_null("starts_at"))
                    .col(schema::timestamp_with_time_zone_null("ends_at"))
                    .col(schema::integer("priority"))
                    // `draft`, `active`, `paused`.
                    .col(schema::string_len("status", 8))
                    .col(schema::string_len_null("preset_key", 64))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_theme_variants_org_idx")
                    .table("content_theme_variants")
                    .col("tenant_id")
                    .col("org_id")
                    .col("status")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_theme_variants_list_idx")
                    .table("content_theme_variants")
                    .col("tenant_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_theme_variants").to_owned()).await
    }
}
