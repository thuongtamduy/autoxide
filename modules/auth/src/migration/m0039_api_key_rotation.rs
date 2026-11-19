//! Xoay vòng API key (bước 54 P4): key mới ghi key nó thay (`rotated_from`), để audit và giao diện
//! nối được chuỗi key của một máy.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0039_api_key_rotation"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_api_key")
                    .add_column(schema::uuid_null("rotated_from"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter().table("auth_api_key").drop_column("rotated_from").to_owned(),
            )
            .await
    }
}
