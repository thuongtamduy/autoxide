//! Bộ nháp chiến dịch từ một chương trình khuyến mại (bước 67, D-109, D-20261123-campaign-drafts): mỗi
//! yêu cầu là một dòng — chương trình, website, các kênh (trang đích, banner, chiến dịch `newsletter`),
//! ai yêu cầu (ảnh chụp quyền), kết quả của từng kênh. Chạy bằng job: gọi mô hình ngoài transaction.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0033_campaign_drafts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_campaign_drafts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Website (nút) của trang đích và banner; `NULL`: nút của người yêu cầu.
                    .col(schema::uuid_null("org_id"))
                    // Chỉ giữ ID của chương trình (không foreign key xuyên module).
                    .col(schema::uuid("promotion_id"))
                    .col(schema::string_len("locale", 16))
                    // JSON: các kênh `page`, `banner`, `newsletter`.
                    .col(schema::text("channels"))
                    .col(schema::text("instruction"))
                    .col(schema::string_len_null("banner_placement", 64))
                    .col(schema::uuid_null("newsletter_list_id"))
                    // `queued`, `succeeded`, `partial`, `failed`.
                    .col(schema::string_len("status", 10))
                    .col(schema::integer("tries"))
                    .col(schema::string_len_null("error_code", 64))
                    // JSON: kết quả của từng kênh (bản nháp tạo ra, hay mã lỗi).
                    .col(schema::text_null("results"))
                    // Người yêu cầu (ảnh chụp quyền), để job tạo nháp với đúng quyền đó.
                    .col(schema::text("principal"))
                    .col(schema::uuid("requested_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_campaign_drafts_requester_idx")
                    .table("content_campaign_drafts")
                    .col("tenant_id")
                    .col("requested_by")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_campaign_drafts").to_owned()).await
    }
}
