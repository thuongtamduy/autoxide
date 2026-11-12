//! Thông tin để người dùng nhận ra từng phiên đăng nhập của mình: thiết bị
//! (`user_agent`) và lúc phiên bắt đầu (`session_started_at`). Mỗi refresh token mới của
//! một họ chép lại hai giá trị này từ token trước.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0015_add_session_details"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Mỗi lệnh `ALTER TABLE` một cột: SQLite không nhận nhiều thay đổi trong một lệnh.
        // Cột cho phép NULL: phiên có sẵn không biết thiết bị, lúc bắt đầu lấy theo token.
        manager
            .alter_table(
                Table::alter()
                    .table("auth_refresh_token")
                    .add_column(schema::string_len_null("user_agent", 255))
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table("auth_refresh_token")
                    .add_column(schema::timestamp_with_time_zone_null("session_started_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["session_started_at", "user_agent"] {
            manager
                .alter_table(
                    Table::alter().table("auth_refresh_token").drop_column(column).to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
