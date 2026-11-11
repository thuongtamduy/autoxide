//! Bảng `auth_mfa_challenge`: lượt đăng nhập đã qua mật khẩu, đang chờ lớp thứ hai.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_create_mfa_challenge"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_mfa_challenge")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("user_id"))
                    // SHA-256 của `mfa_token` trả cho client. Không lưu token gốc.
                    .col(schema::string_len("token_hash", 64))
                    .col(schema::integer("attempts"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_mfa_challenge_user_fk")
                            .from("auth_mfa_challenge", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_mfa_challenge_hash_unique")
                    .table("auth_mfa_challenge")
                    .col("token_hash")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_mfa_challenge").to_owned()).await
    }
}
