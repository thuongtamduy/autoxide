//! Index `audit_entry(module, id)`: lọc nhật ký theo module (`?module=…`) đi theo index thay vì
//! duyệt cả bảng — bảng chỉ lớn thêm, không bao giờ dọn (D-055, D-158).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0003_add_module_index"
    }
}

const INDEX: &str = "audit_entry_module_idx";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create().name(INDEX).table("audit_entry").col("module").col("id").to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(INDEX).table("audit_entry").to_owned()).await
    }
}
