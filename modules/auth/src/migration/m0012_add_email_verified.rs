//! Cột `auth_user.email_verified_at`: lúc chủ tài khoản xác nhận email là của mình.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_add_email_verified"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Cột cho phép NULL: SQLite và Postgres đều thêm được bằng `ALTER TABLE … ADD`.
        // Tài khoản có sẵn coi như chưa xác minh.
        manager
            .alter_table(
                Table::alter()
                    .table("auth_user")
                    .add_column(schema::timestamp_with_time_zone_null("email_verified_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("auth_user").drop_column("email_verified_at").to_owned(),
            )
            .await
    }
}
