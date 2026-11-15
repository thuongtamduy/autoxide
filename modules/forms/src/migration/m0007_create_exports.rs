//! Bảng `forms_exports`: bản xuất CSV chạy bằng job (đặc tả 32 B3). Không foreign key, như mọi
//! bảng của module; file nằm ở nơi lưu chung (`dyn Storage`), khoá `forms/exports/<id>.csv`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_create_exports"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("forms_exports")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("form_id"))
                    .col(schema::uuid_null("requested_by"))
                    .col(schema::string_len_null("status_filter", 16))
                    .col(schema::string_len("state", 16))
                    .col(schema::big_integer_null("row_count"))
                    .col(schema::big_integer_null("size"))
                    .col(schema::string_len_null("storage_key", 255))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("finished_at"))
                    .col(schema::timestamp_with_time_zone_null("expires_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("forms_exports_form_idx")
                    .table("forms_exports")
                    .col("form_id")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("forms_exports_expires_idx")
                    .table("forms_exports")
                    .col("expires_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("forms_exports").to_owned()).await
    }
}
