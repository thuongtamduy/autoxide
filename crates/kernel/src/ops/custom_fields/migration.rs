//! Migration tạo `kernel_custom_fields`.

use super::*;

/// Migration tạo `kernel_custom_fields` (cấp khách hàng, mọi database dữ liệu).
pub(crate) struct CreateCustomFields;

impl MigrationName for CreateCustomFields {
    fn name(&self) -> &str {
        "m0029_create_custom_fields"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateCustomFields {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new(TABLE))
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::string_len("target", 128))
                    .col(schema::string_len("key", 64))
                    .col(schema::string_len("data_type", 16))
                    .col(schema::text("labels"))
                    .col(schema::text("help"))
                    .col(schema::boolean("required"))
                    .col(schema::text("options"))
                    .col(schema::integer_null("scale"))
                    .col(schema::string_len_null("min_value", 64))
                    .col(schema::string_len_null("max_value", 64))
                    .col(schema::string_len_null("reference_kind", 128))
                    .col(schema::text_null("default_value"))
                    .col(schema::string_len("visibility", 16))
                    .col(schema::boolean("filterable"))
                    .col(schema::boolean("personal_data"))
                    .col(schema::integer("position"))
                    .col(schema::boolean("active"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("kernel_custom_fields_key")
                    .table(Alias::new(TABLE))
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("target"))
                    .col(Alias::new("key"))
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new(TABLE)).to_owned()).await
    }
}
