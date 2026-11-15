//! Cách đăng nhập và quốc gia gần đúng của mỗi phiên (đề bài giao diện D, D-272), để người dùng
//! nhận ra phiên lạ. `sign_in_method`: `password`, `password_and_mfa`, `passkey`,
//! `password_change`. `country`: mã ISO 3166-1 hai chữ lấy từ header của proxy tin cậy (vd
//! `CF-IPCountry`), không bao giờ là địa chỉ IP; mất cùng dòng khi phiên hết hạn và bị dọn. Mỗi
//! refresh token mới của một họ chép lại hai giá trị này. Phiên có sẵn: `NULL`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0024_session_sign_in"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Mỗi lệnh `ALTER TABLE` một cột: SQLite không nhận nhiều thay đổi trong một lệnh.
        for column in
            [schema::string_len_null("sign_in_method", 32), schema::string_len_null("country", 2)]
        {
            manager
                .alter_table(
                    Table::alter().table("auth_refresh_token").add_column(column).to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["country", "sign_in_method"] {
            manager
                .alter_table(
                    Table::alter().table("auth_refresh_token").drop_column(column).to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}
