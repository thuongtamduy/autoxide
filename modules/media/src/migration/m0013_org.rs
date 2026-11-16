//! Item thuộc một nút của cây tổ chức (bước 34, D-208): cửa hàng, người bán trên sàn có thư viện
//! riêng. `org_id`: item cũ ở nút gốc (= ID của khách). Cột NULL được ở database, điền ngay ở đây
//! và service luôn ghi (D-304 mục 6). Thư mục và nhãn giữ cấp khách.

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, Statement};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_org"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("media_item")
                    .add_column(schema::uuid_null("org_id"))
                    .to_owned(),
            )
            .await?;
        let connection = manager.get_connection();
        let backend = connection.get_database_backend();
        connection
            .execute_raw(Statement::from_string(
                backend,
                "UPDATE media_item SET org_id = tenant_id".to_owned(),
            ))
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("media_item_org_idx")
                    .table("media_item")
                    .col("tenant_id")
                    .col("org_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name("media_item_org_idx").table("media_item").to_owned())
            .await?;
        manager
            .alter_table(Table::alter().table("media_item").drop_column("org_id").to_owned())
            .await
    }
}
