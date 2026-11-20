//! Tài khoản người mua chỉ có số điện thoại (bước 47, D-20261120-phone-accounts): `email` vẫn NOT NULL,
//! tài khoản không có email giữ `''`. Unique email thành index một phần (`WHERE email <> ''`) để nhiều
//! tài khoản không email cùng tồn tại; thêm `phone_key` (HMAC của số theo khách, `SecretKey::phone_key`)
//! unique theo (khách, realm) — NULL không đụng nhau trên cả SQLite và Postgres.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0043_phone_accounts"
    }
}

const EMAIL_UNIQUE: &str = "auth_user_email_unique";
const PHONE_UNIQUE: &str = "auth_user_phone_unique";

fn index(name: &str, columns: &[&str]) -> IndexCreateStatement {
    let mut index = Index::create();
    index.name(name).table(Alias::new("auth_user")).unique();
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
                    .add_column(schema::string_len_null("phone_key", 64))
                    .to_owned(),
            )
            .await?;
        manager.drop_index(Index::drop().name(EMAIL_UNIQUE).table("auth_user").to_owned()).await?;
        let mut email = index(EMAIL_UNIQUE, &["tenant_id", "realm", "email"]);
        email.and_where(Expr::col(Alias::new("email")).ne(""));
        manager.create_index(email).await?;
        manager.create_index(index(PHONE_UNIQUE, &["tenant_id", "realm", "phone_key"])).await
    }

    // Lùi lại khi đã có hai tài khoản không email cùng khách và realm thì vấp unique: xoá trước.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_index(Index::drop().name(PHONE_UNIQUE).table("auth_user").to_owned()).await?;
        manager.drop_index(Index::drop().name(EMAIL_UNIQUE).table("auth_user").to_owned()).await?;
        manager.create_index(index(EMAIL_UNIQUE, &["tenant_id", "realm", "email"])).await?;
        manager
            .alter_table(Table::alter().table("auth_user").drop_column("phone_key").to_owned())
            .await
    }
}
