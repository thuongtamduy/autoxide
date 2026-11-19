//! Giỏ hàng (`orders_carts`) và dòng giỏ (`orders_cart_lines`). Mọi index có `tenant_id` đứng đầu
//! (D-185); không foreign key sang bảng của module khác — chỉ giữ ID.

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0005_carts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("orders_carts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Nút bán (cửa hàng / gian hàng).
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid_null("buyer_user_id"))
                    // Vãng lai: SHA-256 (hex) của `Cart-Token`; token thô không bao giờ lưu.
                    .col(schema::string_len_null("token_hash", 64))
                    .col(schema::string_len("currency", 3))
                    .col(schema::string_len("locale", 16))
                    // JSON: coupon (`codes`), voucher trong ví (`voucher_ids`), quà đã chọn.
                    .col(schema::text("codes"))
                    .col(schema::text("voucher_ids"))
                    .col(schema::text("gift_selections"))
                    // JSON: `{"address_id"}` hay `{"address", "recipient_name", "phone"}`.
                    .col(schema::text_null("ship_to"))
                    .col(schema::string_len_null("shipping_option", 200))
                    .col(schema::string_len_null("payment_method", 32))
                    .col(schema::text_null("attribution"))
                    .col(schema::string_len("status", 16))
                    .col(schema::uuid_null("merged_into"))
                    // `<org_id>:<buyer_user_id>` khi giỏ `active` của người mua, NULL khi không:
                    // unique trên cột NULL thay cho index một phần (đặc tả 45 "Unique có NULL").
                    .col(schema::string_len_null("active_buyer_key", 80))
                    .col(schema::timestamp_with_time_zone_null("abandoned_notified_at"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .col(schema::timestamp_with_time_zone("expires_at"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_carts";
        index(manager, t, "orders_carts_buyer_unique", &["tenant_id", "active_buyer_key"], true)
            .await?;
        index(manager, t, "orders_carts_token_idx", &["tenant_id", "token_hash"], false).await?;
        index(manager, t, "orders_carts_status_idx", &["tenant_id", "status", "updated_at"], false)
            .await?;
        index(manager, t, "orders_carts_buyer_idx", &["tenant_id", "buyer_user_id"], false).await?;
        index(manager, t, "orders_carts_expires_idx", &["tenant_id", "expires_at"], false).await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_cart_lines")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("cart_id"))
                    .col(schema::uuid("variant_id"))
                    .col(schema::big_integer("quantity"))
                    .col(schema::timestamp_with_time_zone("added_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("cart_id").col("variant_id"))
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["orders_cart_lines", "orders_carts"] {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
