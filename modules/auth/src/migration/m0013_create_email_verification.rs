//! Bảng `auth_email_verification`: token xác minh email đã gửi qua mail.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_create_email_verification"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_email_verification")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("user_id"))
                    // Địa chỉ cần xác minh: email hiện tại, hoặc email mới khi đổi email.
                    .col(schema::string_len("email", 254))
                    // SHA-256 của token gửi trong mail. Không lưu token gốc.
                    .col(schema::string_len("token_hash", 64))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("used_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_email_verification_user_fk")
                            .from("auth_email_verification", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_email_verification_hash_unique")
                    .table("auth_email_verification")
                    .col("token_hash")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_email_verification_user_idx")
                    .table("auth_email_verification")
                    .col("user_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_email_verification").to_owned()).await
    }
}
