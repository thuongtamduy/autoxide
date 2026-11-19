//! Câu trả lời mẫu (macro) theo nút, kế thừa xuống nhánh (bước 53 mục 4); cột duyệt cho trả lời
//! tự động dùng ở mục 6 (D-230).

use autoxide_kernel::migration::prelude::*;

use super::m0001_create_support::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_macros"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("support_macros")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("name", 100))
                    // JSON: nội dung theo ngôn ngữ; cách hỏi mẫu theo ngôn ngữ.
                    .col(schema::text("bodies"))
                    .col(schema::boolean("auto_reply"))
                    .col(schema::text("questions"))
                    .col(schema::uuid_null("approved_by"))
                    .col(schema::timestamp_with_time_zone_null("approved_at"))
                    // Ngày phải rà lại (`YYYY-MM-DD`), quá ngày thì thôi trả lời tự động.
                    .col(schema::string_len_null("review_due_on", 10))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        index(manager, "support_macros", "support_macros_org_idx", &["tenant_id", "org_id"], false)
            .await?;
        index(
            manager,
            "support_macros",
            "support_macros_auto_idx",
            &["tenant_id", "org_id", "auto_reply"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("support_macros").to_owned()).await
    }
}
