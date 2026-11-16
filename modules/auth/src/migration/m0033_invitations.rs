//! Lời mời người dùng qua email (bước 34 mục 12, D-312): quản trị viên mời kèm role định sẵn ở một
//! nút; người được mời chấp nhận bằng link dùng một lần. Role giữ dạng danh sách JSON trong dòng
//! lời mời (một bảng thay hai: không ai truy vấn theo từng role của lời mời).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0033_invitations"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_invitation")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::string_len("email", 254))
                    .col(schema::string_len("name", 200))
                    .col(schema::uuid("org_id"))
                    // JSON: mảng ID role.
                    .col(schema::text("role_ids"))
                    .col(schema::string_len_null("reference", 200))
                    .col(schema::uuid_null("invited_by"))
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::uuid_null("accepted_user_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("auth_invitation_status_idx", ["tenant_id", "status", "created_at"]),
            ("auth_invitation_email_idx", ["tenant_id", "email", "id"]),
        ] {
            let mut index = Index::create();
            index.name(name).table("auth_invitation");
            for column in columns {
                index.col(column);
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_invitation").to_owned()).await
    }
}
