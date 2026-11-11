//! Bảng `files_file`: thông tin của mỗi file đã tải lên. Nội dung nằm ở nơi lưu
//! (`Storage`), khoá `files/<id>`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_file"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("files_file")
                    .col(schema::pk_uuid("id"))
                    // Người tải lên. Chỉ giữ ID, không foreign key sang module `auth`.
                    .col(schema::uuid("owner_id"))
                    .col(schema::string_len("name", 255))
                    .col(schema::string_len("content_type", 255))
                    .col(schema::big_integer("size"))
                    // SHA-256 của nội dung, dạng hex: client kiểm file tải về còn nguyên.
                    .col(schema::string_len("sha256", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        // Danh sách file của một người, phân trang theo ID.
        manager
            .create_index(
                Index::create()
                    .name("files_file_owner_idx")
                    .table("files_file")
                    .col("owner_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("files_file").to_owned()).await
    }
}
