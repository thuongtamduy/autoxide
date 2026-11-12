//! Số đếm đăng nhập sai chuyển sang bảng chung của kernel (`auth_throttle`, D-110). Số đếm
//! cũ không được chép: chúng sống tối đa một cửa sổ (mặc định 15 phút).

use autoxide_kernel::migration::prelude::*;
use autoxide_kernel::throttle::throttle_migration;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0018_move_login_throttle"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        throttle_migration("m0018_move_login_throttle", "auth").up(manager).await?;
        manager.drop_table(Table::drop().table("auth_login_throttle").to_owned()).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Dựng lại bảng cũ đúng như m0004 rồi bỏ bảng mới.
        super::m0004_create_login_throttle::Migration.up(manager).await?;
        throttle_migration("m0018_move_login_throttle", "auth").down(manager).await
    }
}
