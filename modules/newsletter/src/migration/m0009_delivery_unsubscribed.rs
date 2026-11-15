//! Cột `unsubscribed_at` của `newsletter_deliveries`: người nhận huỷ đăng ký bằng link của chính
//! thư này (huỷ một chạm hay trang lựa chọn) — để đếm số huỷ của từng chiến dịch (đề bài giao diện
//! B7, D-269). Lượt gửi có sẵn: `NULL` (không biết).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_delivery_unsubscribed"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("newsletter_deliveries")
                    .add_column(schema::timestamp_with_time_zone_null("unsubscribed_at"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("newsletter_deliveries")
                    .drop_column("unsubscribed_at")
                    .to_owned(),
            )
            .await
    }
}
