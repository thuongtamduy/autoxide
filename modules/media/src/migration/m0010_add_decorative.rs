//! Cột `decorative` của `media_item`: ảnh chỉ để trang trí — `alt` để trống có chủ ý, khác "chưa
//! có alt" (đề bài giao diện B5, D-266). Tệp có sẵn: `false`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_add_decorative"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("media_item")
                    .add_column(schema::boolean("decorative").default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("media_item").drop_column("decorative").to_owned())
            .await
    }
}
