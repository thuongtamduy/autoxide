//! Rào quyền tối đa cho khách con (bước 54 P4): bảng ở database nền tảng, một dòng mỗi cặp (khách
//! bị rào, khách đặt rào).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0041_tenant_guardrails"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_tenant_guardrails")
                    .if_not_exists()
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("set_by_tenant_id"))
                    .col(schema::text("max_permissions"))
                    .col(schema::uuid("updated_by"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_tenant_guardrails_pair_idx")
                    .table("auth_tenant_guardrails")
                    .col("tenant_id")
                    .col("set_by_tenant_id")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_tenant_guardrails_set_by_idx")
                    .table("auth_tenant_guardrails")
                    .col("set_by_tenant_id")
                    .col("tenant_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_tenant_guardrails").to_owned()).await
    }
}
