//! Cột `updated_by` của `menus_menus`: người tạo hay thay cây mục gần nhất (đề bài giao diện A6,
//! D-264). Menu có sẵn để `NULL` (không biết ai sửa).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_add_updated_by"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("menus_menus")
                    .add_column(schema::uuid_null("updated_by"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("menus_menus").drop_column("updated_by").to_owned())
            .await
    }
}
