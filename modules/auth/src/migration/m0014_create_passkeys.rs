//! Bảng passkey (D-077):
//!
//! - `auth_passkey`: mỗi dòng một passkey đã đăng ký. Lưu khoá CÔNG KHAI và trạng thái do
//!   thư viện WebAuthn mã hoá thành byte; không có secret nào.
//! - `auth_passkey_ceremony`: nghi thức đăng ký / đăng nhập đang dở (challenge đã gửi
//!   cho trình duyệt). Lưu trong database để máy chủ khác hoàn tất được; dùng một lần,
//!   hết hạn sau vài phút, job dọn dẹp xoá dòng cũ.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0014_create_passkeys"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_passkey")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("user_id"))
                    // Credential ID do authenticator đặt (tối đa 1023 byte), duy nhất.
                    .col(schema::blob("credential_id"))
                    // User handle WebAuthn của người dùng: giống nhau cho mọi passkey của
                    // cùng một người.
                    .col(schema::blob("user_handle"))
                    .col(schema::blob("static_state"))
                    .col(schema::blob("dynamic_state"))
                    .col(schema::string_len("name", 100))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_passkey_user_fk")
                            .from("auth_passkey", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_passkey_credential_unique")
                    .table("auth_passkey")
                    .col("credential_id")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_passkey_user_idx")
                    .table("auth_passkey")
                    .col("user_id")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_passkey_ceremony")
                    .col(schema::pk_uuid("id"))
                    // Người đăng ký passkey; NULL khi là nghi thức đăng nhập.
                    .col(schema::uuid_null("user_id"))
                    .col(schema::blob("state"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_passkey_ceremony_user_fk")
                            .from("auth_passkey_ceremony", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_passkey_ceremony_expires_idx")
                    .table("auth_passkey_ceremony")
                    .col("expires_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_passkey_ceremony").to_owned()).await?;
        manager.drop_table(Table::drop().table("auth_passkey").to_owned()).await
    }
}
