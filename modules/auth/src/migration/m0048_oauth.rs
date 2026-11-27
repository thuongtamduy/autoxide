//! Máy chủ uỷ quyền OAuth 2.1 cho MCP (bước 71 G4, D-20261126-mcp-oauth): client đăng ký động, grant (một lần người
//! dùng duyệt cho một client), access / refresh token của grant. Bảng cấp khách: khoá chính `(tenant_id, id)`. Không
//! foreign key — token xoá cùng grant trong transaction của lệnh thu hồi.

use autoxide_kernel::migration::prelude::*;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0048_oauth"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Client đăng ký động (RFC 7591). Client nhận diện bằng tài liệu metadata (CIMD) không lưu ở đây.
        manager
            .create_table(
                Table::create()
                    .table("auth_oauth_clients")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    // `client_id` cấp cho client (`axdc_<id>`).
                    .col(schema::string_len("client_id", 64))
                    .col(schema::string_len("client_name", 200))
                    // Danh sách `redirect_uris`, JSON.
                    .col(schema::text("redirect_uris"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    // Lần gần nhất client mở `/authorize` (ghi thưa, mỗi ngày một lần): dọn client bỏ rơi.
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_oauth_clients_client_id_unique")
                    .table("auth_oauth_clients")
                    .col("tenant_id")
                    .col("client_id")
                    .unique()
                    .to_owned(),
            )
            .await?;
        // Dọn client chưa từng có grant: theo thời điểm tạo.
        manager
            .create_index(
                Index::create()
                    .name("auth_oauth_clients_created_idx")
                    .table("auth_oauth_clients")
                    .col("tenant_id")
                    .col("created_at")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_oauth_grants")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("user_id"))
                    .col(schema::string_len("client_id", 2048))
                    // Tên client lúc duyệt (dữ liệu chưa tin, chỉ để hiển thị) và cách đăng ký (`cimd`, `dynamic`).
                    .col(schema::string_len("client_name", 200))
                    .col(schema::string_len("registration", 16))
                    .col(schema::string_len("redirect_uri", 2048))
                    .col(schema::string_len("resource", 2048))
                    // PKCE `code_challenge` (S256) của lần duyệt: kiểm khi đổi mã lấy token.
                    .col(schema::string_len("code_challenge", 128))
                    // Permission người dùng đã chọn, cách nhau bằng dấu cách (dạng `scope` của OAuth).
                    .col(schema::text("scope"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("last_used_at"))
                    .col(schema::timestamp_with_time_zone_null("revoked_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        // Danh sách grant của một người, mới nhất trước (keyset theo `id` UUIDv7). Danh sách mọi grant của khách
        // dùng khoá chính `(tenant_id, id)`.
        manager
            .create_index(
                Index::create()
                    .name("auth_oauth_grants_user_idx")
                    .table("auth_oauth_grants")
                    .col("tenant_id")
                    .col("user_id")
                    .col("id")
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("auth_oauth_tokens")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("id"))
                    .col(schema::uuid("grant_id"))
                    // `code` (mã uỷ quyền, sống 60 giây), `access` hay `refresh`.
                    .col(schema::string_len("kind", 16))
                    // SHA-256 của token: lộ bảng cũng không dùng được token.
                    .col(schema::string_len("token_hash", 64))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    // Refresh token đã đổi lấy token mới (xoay vòng).
                    .col(schema::timestamp_with_time_zone_null("used_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("id"))
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_oauth_tokens_hash_unique")
                    .table("auth_oauth_tokens")
                    .col("tenant_id")
                    .col("token_hash")
                    .unique()
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_oauth_tokens_sweep_idx")
                    .table("auth_oauth_tokens")
                    .col("expires_at")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_oauth_clients_sweep_idx")
                    .table("auth_oauth_clients")
                    .col("last_used_at")
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("auth_oauth_tokens_grant_idx")
                    .table("auth_oauth_tokens")
                    .col("tenant_id")
                    .col("grant_id")
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("auth_oauth_tokens").to_owned()).await?;
        manager.drop_table(Table::drop().table("auth_oauth_grants").to_owned()).await?;
        manager.drop_table(Table::drop().table("auth_oauth_clients").to_owned()).await
    }
}
