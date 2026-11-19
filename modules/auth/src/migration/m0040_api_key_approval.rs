//! Quản trị duyệt API key (bước 54 P4): khách bật `auth.api_keys.require_approval` thì key mới ở
//! `pending` tới khi người có `auth.api_keys.approve` duyệt. Key có từ trước là `active`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0040_api_key_approval"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi câu `ALTER TABLE`.
        let columns = [
            schema::string_len("status", 16).default("active").to_owned(),
            schema::uuid_null("approved_by"),
            schema::timestamp_with_time_zone_null("approved_at"),
        ];
        for column in columns {
            manager
                .alter_table(Table::alter().table("auth_api_key").add_column(column).to_owned())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for column in ["approved_at", "approved_by", "status"] {
            manager
                .alter_table(Table::alter().table("auth_api_key").drop_column(column).to_owned())
                .await?;
        }
        Ok(())
    }
}
