//! Bảng `media_alt_translations` (D-202): `alt` theo ngôn ngữ. Khoá chính (`media_id`, `locale`);
//! không foreign key (như mọi bảng của module) — xoá hẳn item thì service xoá bản dịch cùng
//! transaction. Chưa có `tenant_id`: bảng `media_item` chưa theo khách (bước 33 thêm cho cả hai).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_alt_translations"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("media_alt_translations")
                    .col(schema::uuid("media_id"))
                    .col(schema::string_len("locale", 35))
                    .col(schema::text("alt"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("media_id").col("locale"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("media_alt_translations").to_owned()).await
    }
}
