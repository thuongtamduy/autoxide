//! Bảng `auth_recovery_code`: mã khôi phục dùng một lần khi mất thiết bị TOTP.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_create_recovery_codes"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_recovery_code")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("user_id"))
                    // SHA-256 của mã đã chuẩn hoá. Không lưu mã gốc.
                    .col(schema::string_len("code_hash", 64))
                    .col(schema::timestamp_with_time_zone_null("used_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_recovery_code_user_fk")
                            .from("auth_recovery_code", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_recovery_code_user_idx")
                    .table("auth_recovery_code")
                    .col("user_id")
                    .col("code_hash")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_recovery_code").to_owned()).await
    }
}
