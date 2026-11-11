//! Bảng API key: `auth_api_key` và permission của từng key (`auth_api_key_permission`).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_create_api_keys"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_api_key")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("name", 100))
                    // Vài ký tự đầu của key (`axk_` + 8 ký tự) để người dùng nhận ra key nào
                    // là key nào; không đủ để đoán phần còn lại.
                    .col(schema::string_len("prefix", 16))
                    // SHA-256 của key (base64url). Không lưu key gốc.
                    .col(schema::string_len("key_hash", 64))
                    .col(schema::timestamp_with_time_zone_null("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_api_key_user_fk")
                            .from("auth_api_key", "user_id")
                            .to("auth_user", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_api_key_permission")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("api_key_id"))
                    .col(schema::string_len("permission", 200))
                    .foreign_key(
                        ForeignKey::create()
                            .name("auth_api_key_permission_key_fk")
                            .from("auth_api_key_permission", "api_key_id")
                            .to("auth_api_key", "id")
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        for (name, table, columns) in [
            ("auth_api_key_hash_unique", "auth_api_key", &["key_hash"][..]),
            ("auth_api_key_user_idx", "auth_api_key", &["user_id"][..]),
            (
                "auth_api_key_permission_unique",
                "auth_api_key_permission",
                &["api_key_id", "permission"][..],
            ),
        ] {
            let mut index = Index::create();
            index.name(name).table(table);
            for column in columns {
                index.col(*column);
            }
            if name.ends_with("_unique") {
                index.unique();
            }
            manager.create_index(index.to_owned()).await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["auth_api_key_permission", "auth_api_key"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
