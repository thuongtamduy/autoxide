//! Trang chi tiết dựng sẵn của loại nội dung tự định nghĩa (bước 67, D-235): cột `detail_layout` của
//! `content_types` — bố cục khối (JSON trong `text`) mọi mục của loại có trang dùng chung, với khối
//! `content.entry_field` hiện một trường của mục đang xem. `NULL`: không có bố cục mẫu.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0031_type_detail_layout"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("content_types")
                    .add_column(schema::text_null("detail_layout"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("content_types").drop_column("detail_layout").to_owned(),
            )
            .await
    }
}
