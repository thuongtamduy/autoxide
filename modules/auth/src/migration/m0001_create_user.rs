//! Bảng `auth_user`: tài khoản người dùng.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_user"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_user")
                    .col(schema::pk_uuid("id"))
                    // 254: độ dài tối đa của một địa chỉ email (RFC 5321).
                    .col(schema::string_len("email", 254))
                    .col(schema::string_len("name", 200))
                    // Chuỗi PHC của Argon2id (`$argon2id$v=19$m=…`). `NULL`: tài khoản
                    // chưa đặt mật khẩu, không đăng nhập bằng mật khẩu được.
                    .col(schema::text_null("password_hash"))
                    .col(schema::timestamp_with_time_zone_null("disabled_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        // Email đã chuẩn hoá về chữ thường trước khi lưu, nên index thường đủ để
        // chặn trùng không phân biệt hoa thường — trên cả hai database.
        manager
            .create_index(
                Index::create()
                    .name("auth_user_email_unique")
                    .table("auth_user")
                    .col("email")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_user").to_owned()).await
    }
}
