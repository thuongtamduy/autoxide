//! Danh tính hai lớp (bước 34 mục 8, D-309): `auth_user.realm` — `staff` (nhân sự, có role) hay
//! `customer` (người mua của khách, không permission nào). Cùng một email có thể là nhân sự và
//! người mua ở cùng khách: unique email thành `(tenant_id, realm, email)`. Tài khoản cũ là nhân sự.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0031_realm"
    }
}

const UNIQUE: &str = "auth_user_email_unique";

fn unique(columns: &[&str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(UNIQUE).table(Alias::new("auth_user")).unique();
    for column in columns {
        index.col(Alias::new(*column));
    }
    index.to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_user")
                    .add_column(schema::string_len("realm", 16).default("staff"))
                    .to_owned(),
            )
            .await?;
        manager.drop_index(Index::drop().name(UNIQUE).table("auth_user").to_owned()).await?;
        manager.create_index(unique(&["tenant_id", "realm", "email"])).await
    }

    // Lùi lại khi một email đã có cả hai realm trong một khách thì vấp unique: xoá một bên trước.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(UNIQUE).table("auth_user").to_owned()).await?;
        manager.create_index(unique(&["tenant_id", "email"])).await?;
        manager.alter_table(Table::alter().table("auth_user").drop_column("realm").to_owned()).await
    }
}
