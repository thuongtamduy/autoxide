//! Tài khoản (bước 34 mục 9, D-308): ngôn ngữ và múi giờ ưa thích của người dùng (mail gửi chậm
//! dùng chúng thay ngôn ngữ của request gốc), role bắt buộc MFA (quyền của role chỉ có hiệu lực
//! khi tài khoản đã bật MFA).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0030_account"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in
            [schema::string_len_null("locale", 35), schema::string_len_null("time_zone", 64)]
        {
            manager
                .alter_table(Table::alter().table("auth_user").add_column(column).to_owned())
                .await?;
        }
        manager
            .alter_table(
                Table::alter()
                    .table("auth_role")
                    .add_column(schema::boolean("requires_mfa").default(false))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("auth_role").drop_column("requires_mfa").to_owned())
            .await?;
        for column in ["locale", "time_zone"] {
            manager
                .alter_table(Table::alter().table("auth_user").drop_column(column).to_owned())
                .await?;
        }
        Ok(())
    }
}
