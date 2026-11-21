//! Đồng bộ nhân viên qua SCIM 2.0 (bước 66, D-20261121-scim): token SCIM của khách và liên kết tài
//! khoản nhân sự với người dùng ở hệ thống danh tính của khách (Entra ID, Okta, Google). Bảng cấp
//! khách: khoá chính `(tenant_id, id)`. Không foreign key — liên kết xoá cùng tài khoản trong
//! transaction của lệnh xoá (như `auth_identity`).

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0044_scim"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_scim_tokens")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::string_len("name", 100))
                    // Phần nhận diện để người quản trị nhận ra token (`axscim1_…abcd`).
                    .col(schema::string_len("prefix", 32))
                    // SHA-256 của token: lộ bảng cũng không dùng được token.
                    .col(schema::string_len("token_hash", 64))
                    .col(schema::uuid("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_scim_tokens_hash_unique")
                    .table("auth_scim_tokens")
                    .col("token_hash")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_scim_users")
                    .col(schema::uuid("tenant_id"))
                    // Chính ID tài khoản (`auth_user.id`): một tài khoản một liên kết.
                    .col(schema::uuid("id"))
                    .col(schema::string_len("user_name", 255))
                    // `userName` viết thường: unique và lọc không phân biệt hoa thường (RFC 7643).
                    .col(schema::string_len("user_name_key", 255))
                    .col(schema::string_len_null("external_id", 255))
                    // `externalId` viết thường, cho lọc `co` / `sw` / `ew` như nhau trên hai database.
                    .col(schema::string_len_null("external_key", 255))
                    .col(schema::string_len_null("given_name", 200))
                    .col(schema::string_len_null("family_name", 200))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_scim_users_user_name_unique")
                    .table("auth_scim_users")
                    .col("tenant_id")
                    .col("user_name_key")
                    .unique()
                    .to_owned(),
            )
            .await?;
        // NULL không đụng nhau trên cả SQLite và Postgres: nhiều người chưa có `externalId`.
        manager
            .create_index(
                Index::create()
                    .name("auth_scim_users_external_id_unique")
                    .table("auth_scim_users")
                    .col("tenant_id")
                    .col("external_id")
                    .unique()
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_scim_users").to_owned()).await?;
        manager.drop_table(Table::drop().table("auth_scim_tokens").to_owned()).await
    }
}
