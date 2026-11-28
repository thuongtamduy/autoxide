//! Ai chịu trách nhiệm cho dữ liệu SCIM (P16 của bước 70, D-20261128-scim-owners):
//! - `auth_scim_group_members.added_by`: người tạo token SCIM đã đưa người vào nhóm (hay người duyệt). Luật ánh xạ của
//!   nhóm chỉ cấp role cho thành viên mà người này cấp được role đó; còn lại "chờ duyệt".
//! - `auth_scim_users.created_by`: người tạo token SCIM đã tạo (hay nhận về) liên kết của tài khoản. Đồng bộ đưa tài
//!   khoản lên role người này không cấp được thì mọi cách vào cũ của tài khoản bị thu hồi trước.
//!
//! Dòng có từ trước: NULL — không biết ai, coi như không cấp được gì (thành viên chờ duyệt; nâng role thì thu hồi),
//! như `auth_oidc_provider.configured_by` (m0046).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0049_scim_owners"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ nhận một cột mỗi lệnh `ALTER TABLE`: hai lệnh.
        manager
            .alter_table(
                Table::alter()
                    .table("auth_scim_group_members")
                    .add_column(schema::uuid_null("added_by"))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table("auth_scim_users")
                    .add_column(schema::uuid_null("created_by"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("auth_scim_users").drop_column("created_by").to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter().table("auth_scim_group_members").drop_column("added_by").to_owned(),
            )
            .await
    }
}
