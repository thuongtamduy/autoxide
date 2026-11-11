//! Bảng `auth_refresh_token`: refresh token đã cấp, để xoay vòng và phát hiện dùng lại.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0002_create_refresh_token"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_refresh_token")
                    .col(schema::pk_uuid("id"))
                    // Mọi token sinh ra từ cùng một lần đăng nhập chung một `family_id`:
                    // phát hiện dùng lại thì thu hồi cả họ.
                    .col(schema::uuid("family_id"))
                    .col(schema::uuid("user_id"))
                    // SHA-256 của token (base64url, 43 ký tự). Không lưu token gốc.
                    .col(schema::string_len("token_hash", 64))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    // Đã đổi lấy token mới lúc nào; dùng lại token đã đổi là dấu hiệu bị lộ.
                    .col(schema::timestamp_with_time_zone_null("used_at"))
                    .col(schema::timestamp_with_time_zone_null("revoked_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    // Cùng module nên được có foreign key (luật chỉ cấm xuyên module).
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_refresh_token_user_fk")
                            .from("auth_refresh_token", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        for (name, column, unique) in [
            ("auth_refresh_token_hash_unique", "token_hash", true),
            ("auth_refresh_token_family_idx", "family_id", false),
            ("auth_refresh_token_user_idx", "user_id", false),
        ] {
            let mut index = Index::create();
            index.name(name).table("auth_refresh_token").col(column);
            if unique {
                index.unique();
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_refresh_token").to_owned()).await
    }
}
