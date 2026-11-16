//! Hồ sơ công khai của nút (bước 34 mục 7, D-200, D-315): tên hiển thị, địa chỉ (theo quy ước của
//! module `address`), giờ mở cửa, ngày đặc biệt, nhận hàng tại quầy, ảnh bìa. Một bảng riêng: nút
//! không có cửa hàng (vùng, kho) không mang cột trống.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_profiles"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("organizations_profile")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::boolean("public").default(false))
                    .col(schema::integer("position").default(0))
                    .col(schema::string_len_null("display_name", 200))
                    .col(schema::string_len_null("address_country", 2))
                    .col(schema::string_len_null("address_province_code", 10))
                    .col(schema::string_len_null("address_province_name", 200))
                    .col(schema::string_len_null("address_commune_code", 10))
                    .col(schema::string_len_null("address_commune_name", 200))
                    .col(schema::string_len_null("address_line", 300))
                    // JSON đã kiểm.
                    .col(schema::text("opening_hours"))
                    .col(schema::text("special_days"))
                    .col(schema::boolean("pickup").default(false))
                    .col(schema::uuid_null("cover_media_id"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("org_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("organizations_profile_public_idx")
                    .table("organizations_profile")
                    .col("tenant_id")
                    .col("public")
                    .col("position")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("organizations_profile").to_owned()).await
    }
}
