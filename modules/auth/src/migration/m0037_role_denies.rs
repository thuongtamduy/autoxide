//! Deny thắng allow trên role (bước 54, D-205): mỗi dòng `auth_role_permission` có `effect` —
//! `allow` (cấp, như mọi dòng có từ trước) hay `deny` (cấm: không role nào khác của người dùng, kể
//! cả role toàn quyền, cấp lại được). Một cột thay vì bảng riêng để quyền của người gọi vẫn nạp
//! bằng một câu (D-319).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0037_role_denies"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_role_permission")
                    .add_column(schema::string_len("effect", 8).default("allow"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("auth_role_permission").drop_column("effect").to_owned(),
            )
            .await
    }
}
