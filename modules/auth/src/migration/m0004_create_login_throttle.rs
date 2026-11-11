//! Bảng `auth_login_throttle`: đếm lần đăng nhập sai theo email, để khoá tạm việc
//! đoán mật khẩu.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0004_create_login_throttle"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_login_throttle")
                    .col(schema::pk_uuid("id"))
                    // SHA-256 của email đã chuẩn hoá: email lạ (gõ sai, kẻ dò) không bị
                    // lưu lại nguyên văn.
                    .col(schema::string_len("account_key", 64))
                    .col(schema::integer("failures"))
                    .col(schema::timestamp_with_time_zone("window_started_at"))
                    .col(schema::timestamp_with_time_zone_null("locked_until"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_login_throttle_key_unique")
                    .table("auth_login_throttle")
                    .col("account_key")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_login_throttle").to_owned()).await
    }
}
