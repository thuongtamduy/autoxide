//! Cột `restricted_status` của bình luận (bước 37, D-363): bình luận bị ẩn tạm vì vụ việc của
//! `compliance` (báo vi phạm…) giữ trạng thái trước khi ẩn để hiện lại đúng như cũ. `NULL`: không ẩn.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0015_restriction"
    }
}

#[autoxide_kernel::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("comments_comments")
                    .add_column(schema::string_len_null("restricted_status", 16))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("comments_comments")
                    .drop_column("restricted_status")
                    .to_owned(),
            )
            .await
    }
}
