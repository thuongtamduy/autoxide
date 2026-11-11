//! Bảng `auth_totp`: secret TOTP của người dùng đã bật (hoặc đang bật) MFA.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_create_totp"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_totp")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("user_id"))
                    // Secret dạng base32 (RFC 4648), như ứng dụng authenticator nhận.
                    .col(schema::string_len("secret", 64))
                    // `NULL`: đang bật dở, chưa nhập mã xác nhận — chưa có hiệu lực.
                    .col(schema::timestamp_with_time_zone_null("confirmed_at"))
                    // Bước thời gian (30 giây) của mã dùng gần nhất: mã của bước này
                    // hay bước trước đó không được nhận lại (RFC 6238 mục 5.2).
                    .col(schema::big_integer_null("last_used_step"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_totp_user_fk")
                            .from("auth_totp", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_totp_user_unique")
                    .table("auth_totp")
                    .col("user_id")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_totp").to_owned()).await
    }
}
