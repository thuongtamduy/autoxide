//! Mua không cần tài khoản (D-201, D-202): mã một lần gửi qua SMS (`orders_lookup_codes`), token xem
//! đơn (`orders_lookup_tokens`), chống trùng thao tác vãng lai theo `Idempotency-Key`
//! (`orders_guest_requests`) và bảng đếm của `Throttle` (`orders_throttle`). Chỉ lưu bản băm của mã
//! và token. Mọi index có `tenant_id` đứng đầu (D-185).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0010_guests"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("orders_lookup_codes")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    // `lookup` (mở token xem đơn) hay `address` (OTP lần hai để đổi địa chỉ).
                    .col(schema::string_len("purpose", 16))
                    // Token xem đơn đã xin mã `address` (NULL với `lookup`).
                    .col(schema::uuid_null("token_id"))
                    // SHA-256 của (pepper của khách, id, mã).
                    .col(schema::string_len("code_hash", 64))
                    .col(schema::integer("attempts"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone_null("consumed_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "orders_lookup_codes",
            "orders_lookup_codes_order_idx",
            &["tenant_id", "order_id", "purpose", "created_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_lookup_tokens")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    // Khoá liên hệ (`SecretKey::phone_key`) của số đã qua OTP.
                    .col(schema::string_len("phone_hash", 64))
                    // SHA-256 của token (token chỉ trả một lần).
                    .col(schema::string_len("token_hash", 64))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .col(schema::timestamp_with_time_zone("otp_confirmed_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_lookup_tokens";
        index(manager, t, "orders_lookup_tokens_hash_key", &["tenant_id", "token_hash"], true)
            .await?;
        index(manager, t, "orders_lookup_tokens_order_idx", &["tenant_id", "order_id"], false)
            .await?;
        index(manager, t, "orders_lookup_tokens_expiry_idx", &["tenant_id", "expires_at"], false)
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_guest_requests")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    // `cancel`, `address`, `support`.
                    .col(schema::string_len("action", 16))
                    // `Idempotency-Key` của request.
                    .col(schema::string_len("reference", 200))
                    .col(schema::string_len("fingerprint", 64))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(
                        Index::create()
                            .col("tenant_id")
                            .col("order_id")
                            .col("action")
                            .col("reference"),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["orders_guest_requests", "orders_lookup_tokens", "orders_lookup_codes"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
