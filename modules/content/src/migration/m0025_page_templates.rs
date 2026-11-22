//! Trang mẫu (bước 67, D-20261122-page-templates): bố cục khối dựng sẵn theo ngành (khách cài từ bộ
//! mẫu đi kèm bản phát hành) hay trang của khách lưu lại thành mẫu. Tạo trang từ mẫu = chép bố cục.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0025_page_templates"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("content_page_templates")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    // `retail`, `fnb`, `services`, `events`, `recruiting` hay `other`.
                    .col(schema::string_len("industry", 32))
                    .col(schema::string_len("locale", 35))
                    .col(schema::string_len("name", 120))
                    .col(schema::string_len("description", 500))
                    .col(schema::text("layout"))
                    // Khoá trong bộ mẫu đi kèm bản phát hành, khi dòng do bộ đó cài.
                    .col(schema::string_len_null("preset_key", 64))
                    .col(schema::uuid_null("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_page_templates_list_idx")
                    .table("content_page_templates")
                    .col("tenant_id")
                    .col("industry")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("content_page_templates_tenant_id_idx")
                    .table("content_page_templates")
                    .col("tenant_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("content_page_templates").to_owned()).await
    }
}
