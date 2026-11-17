//! Loại API key (bước 51 P12, D-224): `human` (mặc định, key của chính người dùng), `agent` (AI
//! agent) hay `service` (chương trình khác). Audit ghi loại bên làm theo đó; lệnh rủi ro buộc duyệt
//! khi agent gọi. Key có từ trước là `human`.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0035_api_key_kind"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table("auth_api_key")
                    .add_column(schema::string_len("kind", 16).default("human"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table("auth_api_key").drop_column("kind").to_owned())
            .await
    }
}
