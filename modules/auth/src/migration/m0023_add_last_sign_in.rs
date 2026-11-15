//! Cột `last_sign_in_at` của `auth_user`: lúc người dùng mở phiên gần nhất (đề bài giao diện B1,
//! D-265). Tài khoản có sẵn để `NULL` (chưa biết) tới lần đăng nhập sau.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0023_add_last_sign_in"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_user")
                    .add_column(schema::timestamp_with_time_zone_null("last_sign_in_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("auth_user").drop_column("last_sign_in_at").to_owned(),
            )
            .await
    }
}
