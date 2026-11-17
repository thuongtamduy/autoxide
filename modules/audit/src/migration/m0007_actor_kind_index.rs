//! Index `audit_entry(tenant_id, actor_kind, id)` (bước 51 P12, D-224): lọc nhật ký theo loại bên
//! làm (`?actor_kind=agent,service`) đi theo index.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_actor_kind_index"
    }
}

const INDEX: &str = "audit_entry_actor_kind_idx";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name(INDEX)
                    .table("audit_entry")
                    .col("tenant_id")
                    .col("actor_kind")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(INDEX).table("audit_entry").to_owned()).await
    }
}
