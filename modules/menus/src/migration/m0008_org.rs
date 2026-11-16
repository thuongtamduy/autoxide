//! Menu theo website (bước 34, D-238): `org_id` là nút (website) của menu; menu cũ thuộc nút gốc
//! (= ID của khách, website của cả khách). Khoá unique thêm `org_id`: mỗi website một menu `main`
//! riêng. Cột NULL được ở database, điền ngay ở đây và service luôn ghi (D-304 mục 6).

use autoxide_kernel::migration::prelude::*;
use sea_orm::{ConnectionTrait, Statement};

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_org"
    }
}

fn unique(columns: &[&'static str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name("menus_menus_key_locale_unique").table("menus_menus").unique();
    for column in columns {
        index.col(*column);
    }
    index.to_owned()
}

fn drop_unique() -> IndexDropStatement {
    Index::drop().name("menus_menus_key_locale_unique").table("menus_menus").to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("menus_menus")
                    .add_column(schema::uuid_null("org_id"))
                    .to_owned(),
            )
            .await?;
        let connection = manager.get_connection();
        let backend = connection.get_database_backend();
        connection
            .execute_raw(Statement::from_string(
                backend,
                "UPDATE menus_menus SET org_id = tenant_id".to_owned(),
            ))
            .await?;
        manager.drop_index(drop_unique()).await?;
        manager.create_index(unique(&["tenant_id", "org_id", "key", "locale"])).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(drop_unique()).await?;
        manager.create_index(unique(&["tenant_id", "key", "locale"])).await?;
        manager
            .alter_table(Table::alter().table("menus_menus").drop_column("org_id").to_owned())
            .await
    }
}
