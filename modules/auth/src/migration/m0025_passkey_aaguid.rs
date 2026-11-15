//! Cột `aaguid` của `auth_passkey` (đề bài giao diện D, D-272): mã loại authenticator do nó tự
//! khai lúc đăng ký (vd Windows Hello, Apple Passwords), để hiện tên loại cạnh tên người dùng đặt.
//! `NULL`: passkey đăng ký trước cột này, hay authenticator không khai (mã toàn số 0).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0025_passkey_aaguid"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_passkey")
                    .add_column(schema::uuid_null("aaguid"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("auth_passkey").drop_column("aaguid").to_owned())
            .await
    }
}
