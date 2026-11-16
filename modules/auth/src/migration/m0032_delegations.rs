//! Uỷ quyền theo cây khách hàng (bước 34 mục 10, D-310): khách cha cho người giữ một role của mình
//! vào khách con với role đó, ở một nút của khách con. Dòng thuộc khách CHA (`tenant_id`): cha cấp
//! và thu hồi cùng transaction với event của mình; khách con đọc qua database của cha.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0032_delegations"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_delegation")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("child_tenant_id"))
                    .col(schema::uuid("role_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("reason", 500))
                    .col(schema::uuid_null("granted_by"))
                    .col(schema::timestamp_with_time_zone_null("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("revoked_at"))
                    .col(schema::uuid_null("revoked_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_delegation_child_idx")
                    .table("auth_delegation")
                    .col("tenant_id")
                    .col("child_tenant_id")
                    .col("revoked_at")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_delegation").to_owned()).await
    }
}
