//! Bảng `mail_bounces` (D-20261125-mail-bounces): mỗi (khách, địa chỉ) một dòng. Có `tenant_id` từ đầu.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_create_bounces"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("mail_bounces")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Địa chỉ đã chuẩn hoá (chữ thường); độ dài tối đa của một địa chỉ email là 254.
                    .col(schema::string_len("address", 320))
                    .col(schema::integer("hard_count"))
                    // Lần dội cứng tính tới ngưỡng chặn, đặt lại về 0 khi gỡ chặn.
                    .col(schema::integer("strikes"))
                    .col(schema::string_len("last_code", 20))
                    .col(schema::timestamp_with_time_zone("first_at"))
                    .col(schema::timestamp_with_time_zone("last_at"))
                    .col(schema::timestamp_with_time_zone_null("suppressed_at"))
                    // Thư bị bỏ vì địa chỉ đang chặn: đếm và lần cuối (dấu vết tra được, không im lặng).
                    .col(schema::integer("skipped_count"))
                    .col(schema::timestamp_with_time_zone_null("last_skipped_at"))
                    .col(schema::string_len_null("last_skipped_source", 100))
                    .col(schema::timestamp_with_time_zone_null("released_at"))
                    .col(schema::uuid_null("released_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Một dòng cho mỗi địa chỉ của một khách: hai lần dội cùng lúc không tạo hai dòng.
        manager
            .create_index(
                Index::create()
                    .name("mail_bounces_tenant_address_unique")
                    .table("mail_bounces")
                    .col("tenant_id")
                    .col("address")
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Danh sách theo khách, phân trang theo ID.
        manager
            .create_index(
                Index::create()
                    .name("mail_bounces_tenant_id_idx")
                    .table("mail_bounces")
                    .col("tenant_id")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        // Dọn theo thời hạn giữ.
        manager
            .create_index(
                Index::create()
                    .name("mail_bounces_last_at_idx")
                    .table("mail_bounces")
                    .col("last_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("mail_bounces").to_owned()).await
    }
}
