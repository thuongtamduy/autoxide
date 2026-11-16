//! Form có thể thuộc một nút của cây tổ chức (bước 34, D-208): `org_id` NULL được — NULL là form
//! của cả khách (mọi form có sẵn); có giá trị là form của nút đó (cửa hàng, người bán).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0011_org"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("forms_forms")
                    .add_column(schema::uuid_null("org_id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("forms_forms_org_idx")
                    .table("forms_forms")
                    .col("tenant_id")
                    .col("org_id")
                    .col("id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_index(Index::drop().name("forms_forms_org_idx").table("forms_forms").to_owned())
            .await?;
        manager
            .alter_table(Table::alter().table("forms_forms").drop_column("org_id").to_owned())
            .await
    }
}
