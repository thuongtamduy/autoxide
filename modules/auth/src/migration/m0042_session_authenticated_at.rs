//! Xác nhận lại cho việc nhạy cảm (bước 54 P5): mỗi dòng refresh token giữ lần đăng nhập hay xác
//! nhận lại gần nhất của phiên, để access token cấp lại khi refresh vẫn mang đúng `auth_time`.
//! Phiên có từ trước: NULL — đọc là lúc phiên bắt đầu.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0042_session_authenticated_at"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_refresh_token")
                    .add_column(schema::timestamp_with_time_zone_null("authenticated_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_refresh_token")
                    .drop_column("authenticated_at")
                    .to_owned(),
            )
            .await
    }
}
