//! Nhật ký trả lời tự động (bước 53 mục 6, D-230): mỗi tin của người mua được trả lời tự động
//! nhiều nhất một lần (`message_ref` duy nhất), phản hồi "hữu ích / không đúng".

use autoxide_kernel::migration::prelude::*;

use super::m0001_create_support::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0012_auto_replies"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("support_auto_replies")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("thread", 100))
                    .col(schema::string_len("message_ref", 100))
                    .col(schema::string_len("source_kind", 16))
                    .col(schema::uuid_null("macro_id"))
                    .col(schema::string_len_null("intent", 100))
                    .col(schema::integer("confidence_bp"))
                    .col(schema::string_len_null("feedback", 16))
                    .col(schema::uuid_null("reply_message_id"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        let t = "support_auto_replies";
        index(
            manager,
            t,
            "support_auto_replies_message_unique",
            &["tenant_id", "message_ref"],
            true,
        )
        .await?;
        index(
            manager,
            t,
            "support_auto_replies_org_idx",
            &["tenant_id", "org_id", "created_at"],
            false,
        )
        .await?;
        index(
            manager,
            t,
            "support_auto_replies_macro_idx",
            &["tenant_id", "macro_id", "feedback"],
            false,
        )
        .await?;
        index(manager, t, "support_auto_replies_thread_idx", &["tenant_id", "thread"], false).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("support_auto_replies").to_owned()).await
    }
}
