//! Đăng nhập qua nhà cung cấp danh tính (bước 40 P3): nhà cung cấp của khách và liên kết tài khoản
//! với danh tính ở đó. Bảng cấp khách (bước 33): khoá chính `(tenant_id, id)`. Không foreign key —
//! liên kết xoá cùng tài khoản / nhà cung cấp trong transaction của lệnh xoá.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0036_oidc"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("auth_oidc_provider")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::string_len("slug", 64))
                    .col(schema::string_len("kind", 16))
                    .col(schema::string_len("name", 100))
                    .col(schema::string_len_null("issuer", 255))
                    .col(schema::string_len("client_id", 255))
                    // `sealed:v1:<key_id>:…` (AEAD, D-257); rỗng khi client công khai.
                    .col(schema::text("client_secret"))
                    .col(schema::string_len("scopes", 255))
                    // JSON: mảng chuỗi.
                    .col(schema::text("redirect_uris"))
                    .col(schema::text_null("allowed_email_domains"))
                    .col(schema::boolean("allow_signup"))
                    .col(schema::string_len("realm", 16))
                    .col(schema::uuid_null("default_org_id"))
                    .col(schema::uuid_null("default_role_id"))
                    .col(schema::boolean("enabled"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_oidc_provider_slug_unique")
                    .table("auth_oidc_provider")
                    .col("tenant_id")
                    .col("slug")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_identity")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::uuid("provider_id"))
                    .col(schema::string_len("subject", 255))
                    .col(schema::string_len_null("email", 320))
                    .col(schema::boolean("email_verified"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone_null("last_login_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_identity_subject_unique")
                    .table("auth_identity")
                    .col("tenant_id")
                    .col("provider_id")
                    .col("subject")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_identity_user_idx")
                    .table("auth_identity")
                    .col("tenant_id")
                    .col("user_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_identity").to_owned()).await?;
        manager.drop_table(Table::drop().table("auth_oidc_provider").to_owned()).await
    }
}
