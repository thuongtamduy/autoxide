//! Đếm đường dẫn 404 của website (bước 67, D-20261123-site-health-daily): `GET /public/resolve` không
//! thấy gì thì cộng một lượt vào dòng (website, đường dẫn, ngày). Không lưu IP hay gì về người xem; giữ
//! 30 ngày. Kiểm tra sức khoẻ website đọc bảng này để gợi ý chuyển hướng.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0034_not_found_paths"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_not_found_paths")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Website (nút) được hỏi; website của cả khách là nút gốc.
                    .col(schema::uuid("org_id"))
                    // SHA-256 (hex) của ngôn ngữ + đường dẫn: khoá ngắn cố định cho index unique.
                    .col(schema::string_len("path_hash", 64))
                    .col(schema::string_len("locale", 16))
                    .col(schema::string_len("path", 512))
                    // Ngày UTC `YYYY-MM-DD` (chuỗi: so sánh và gộp giống nhau trên cả hai database).
                    .col(schema::string_len("day", 10))
                    .col(schema::big_integer("hits"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_not_found_paths_unique")
                    .table("content_not_found_paths")
                    .col("tenant_id")
                    .col("org_id")
                    .col("path_hash")
                    .col("day")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_not_found_paths_day_idx")
                    .table("content_not_found_paths")
                    .col("tenant_id")
                    .col("org_id")
                    .col("day")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_not_found_paths").to_owned()).await
    }
}
