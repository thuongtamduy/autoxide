//! Quyền vào hỗ trợ của nhân sự nền tảng (bước 34 mục 13, D-313): lần cấp có thời hạn, giữ ở
//! database của khách nền tảng (`tenant_id` = khách nền tảng, `target_tenant_id` = khách được vào).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0034_support_grants"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_support_grant")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("target_tenant_id"))
                    .col(schema::uuid("staff_user_id"))
                    .col(schema::uuid("role_id"))
                    .col(schema::string_len("basis", 16))
                    .col(schema::string_len("reason", 500))
                    .col(schema::string_len_null("legal_reference", 200))
                    .col(schema::uuid_null("granted_by"))
                    .col(schema::timestamp_with_time_zone("starts_at"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("first_used_at"))
                    .col(schema::timestamp_with_time_zone_null("revoked_at"))
                    .col(schema::uuid_null("revoked_by"))
                    .col(schema::boolean("revoked_by_tenant").default(false))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        for (name, columns) in [
            ("auth_support_grant_staff_idx", ["tenant_id", "staff_user_id", "expires_at"]),
            ("auth_support_grant_target_idx", ["tenant_id", "target_tenant_id", "id"]),
        ] {
            let mut index = Index::create();
            index.name(name).table("auth_support_grant");
            for column in columns {
                index.col(column);
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_support_grant").to_owned()).await
    }
}
