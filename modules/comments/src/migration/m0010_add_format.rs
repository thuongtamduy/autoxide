//! Cột `format` của bình luận (D-252): `text` (chữ thường) hay `markdown` (Markdown giới hạn).
//! Dòng có sẵn nhận `text`, nên hiển thị như trước.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_add_format"
    }
}

#[autoxide_kernel::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("comments_comments")
                    .add_column(schema::string_len("format", 16).default("text").to_owned())
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("comments_comments").drop_column("format").to_owned())
            .await
    }
}
