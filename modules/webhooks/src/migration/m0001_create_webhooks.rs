//! Bảng `webhooks_endpoint` (nơi nhận) và `webhooks_delivery` (mỗi lần gửi một event
//! tới một nơi nhận). Mọi bảng của module `webhooks` tên `webhooks_…`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0001_create_webhooks"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("webhooks_endpoint")
                    .col(schema::pk_uuid("id"))
                    .col(schema::string_len("url", 2048))
                    .col(schema::string_len("description", 200))
                    // Cần bản gốc để ký mỗi lần gửi, nên không lưu dạng băm được.
                    .col(schema::string_len("secret", 100))
                    // Danh sách tên event, dạng JSON (`["*"]`: mọi event).
                    .col(schema::text("events"))
                    .col(schema::timestamp_with_time_zone_null("disabled_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("webhooks_delivery")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("endpoint_id"))
                    .col(schema::uuid("event_id"))
                    .col(schema::string_len("event_name", 200))
                    // Thân request đã dựng sẵn: mỗi lần thử gửi đúng cùng một nội dung.
                    .col(schema::text("body"))
                    // `pending`, `succeeded` hoặc `failed`.
                    .col(schema::string_len("status", 16))
                    .col(schema::integer("attempts"))
                    .col(schema::integer_null("response_status"))
                    .col(schema::text_null("last_error"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("last_attempt_at"))
                    .col(schema::timestamp_with_time_zone_null("delivered_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("webhooks_delivery_endpoint_fk")
                            .from("webhooks_delivery", "endpoint_id")
                            .to("webhooks_endpoint", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        // Một event chỉ được gửi tới một nơi nhận một lần, kể cả khi event được giao lại.
        // Index này cũng phục vụ danh sách lần gửi của một nơi nhận.
        manager
            .create_index(
                Index::create()
                    .name("webhooks_delivery_endpoint_event_unique")
                    .table("webhooks_delivery")
                    .col("endpoint_id")
                    .col("event_id")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("webhooks_delivery_endpoint_idx")
                    .table("webhooks_delivery")
                    .col("endpoint_id")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        // Dọn lần gửi cũ theo thời gian.
        manager
            .create_index(
                Index::create()
                    .name("webhooks_delivery_created_idx")
                    .table("webhooks_delivery")
                    .col("created_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("webhooks_delivery").to_owned()).await?;
        manager.drop_table(Table::drop().table("webhooks_endpoint").to_owned()).await
    }
}
