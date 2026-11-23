//! Mốc đầu khung báo tin chưa đọc của từng phía (`notify_customer_from`, `notify_staff_from`): khung
//! bắt đầu từ tin chưa đọc đầu tiên, không theo giờ đồng hồ. Trước đây khoá chống trùng của job tính
//! theo `created_at / khung`, nên vài tin liên tiếp rơi qua ranh giới khung thành hai thông báo.
//! Dòng cũ để trống: tin kế tiếp mở khung mới.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_notify_window"
    }
}

const TABLE: &str = "conversations_threads";
const COLUMNS: [&str; 2] = ["notify_customer_from", "notify_staff_from"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi lệnh ALTER.
        for column in COLUMNS {
            manager
                .alter_table(
                    Table::alter()
                        .table(TABLE)
                        .add_column(schema::timestamp_with_time_zone_null(column))
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in COLUMNS {
            manager.alter_table(Table::alter().table(TABLE).drop_column(column).to_owned()).await?;
        }
        Ok(())
    }
}
