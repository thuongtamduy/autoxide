//! Người cấu hình nhà cung cấp OIDC (D-20261121-oidc-link-covers): người tạo hay sửa gần nhất.
//! Tự liên kết tài khoản theo email chỉ khi người này có mọi permission của tài khoản đó. Dòng có
//! từ trước: NULL — chỉ tự liên kết tài khoản không có permission nào cho tới khi được lưu lại.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0046_oidc_configured_by"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_oidc_provider")
                    .add_column(schema::uuid_null("configured_by"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("auth_oidc_provider").drop_column("configured_by").to_owned(),
            )
            .await
    }
}
