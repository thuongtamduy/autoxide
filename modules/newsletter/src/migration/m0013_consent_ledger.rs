//! Đăng ký bản tin trên sổ đồng ý chung của `compliance` (bước 37 C3, D-367):
//!
//! - `newsletter_subscriptions.ledger_state` (`''`, `granted`, `withdrawn`): sổ đã biết tới đâu. Dòng có
//!   sẵn nhận `''`, nên lịch `newsletter.backfill_consents` ghi sổ cho mọi đăng ký `confirmed` có sẵn
//!   (idempotent theo `source_ref`) — đây là bước chuyển dữ liệu đồng ý cũ sang sổ chung (migration
//!   không gọi được module khác).
//! - `newsletter_campaigns.format` (`static` mặc định — hành vi cũ, hay `template`): chiến dịch cá
//!   nhân hoá bằng bộ dựng mẫu của kernel.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_consent_ledger"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi lệnh `ALTER TABLE`.
        manager
            .alter_table(
                Table::alter()
                    .table("newsletter_subscriptions")
                    .add_column(schema::string_len("ledger_state", 16).default(""))
                    .to_owned(),
            )
            .await?;
        // Lịch đồng bộ quét cả database (mọi khách) tìm đăng ký sổ chưa biết.
        manager
            .create_index(
                Index::create()
                    .name("newsletter_subscriptions_ledger_sweep_idx")
                    .table("newsletter_subscriptions")
                    .col("ledger_state")
                    .col("status")
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table("newsletter_campaigns")
                    .add_column(schema::string_len("format", 16).default("static"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("newsletter_campaigns").drop_column("format").to_owned(),
            )
            .await?;
        manager
            .drop_index(
                Index::drop()
                    .name("newsletter_subscriptions_ledger_sweep_idx")
                    .table("newsletter_subscriptions")
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table("newsletter_subscriptions")
                    .drop_column("ledger_state")
                    .to_owned(),
            )
            .await
    }
}
