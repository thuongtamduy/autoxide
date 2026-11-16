//! Cài đặt kế thừa theo cây (bước 34 mục 4): giá trị một nút đặt cho một cài đặt do module khai
//! (`Module::org_settings`). Nút không đặt dùng giá trị của nút cấp trên gần nhất.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_settings"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("organizations_setting")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("key", 100))
                    // JSON đã kiểm bằng bộ kiểm của cài đặt.
                    .col(schema::text("value"))
                    .col(schema::uuid_null("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("org_id").col("key"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("organizations_setting").to_owned()).await
    }
}
