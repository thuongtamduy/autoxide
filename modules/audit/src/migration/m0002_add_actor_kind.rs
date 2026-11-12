//! Cột `actor_kind` (`system` / `user` / `public`): ai gây ra thay đổi, kể cả người lạ chưa đăng
//! nhập (D-122). Dòng cũ để trống: hiểu theo `actor_user_id`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0002_add_actor_kind"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("audit_entry")
                    .add_column(schema::string_len_null("actor_kind", 16))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("audit_entry").drop_column("actor_kind").to_owned())
            .await
    }
}
