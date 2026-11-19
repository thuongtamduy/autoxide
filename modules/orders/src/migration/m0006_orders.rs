//! Đặt hàng và đơn: lượt đặt (`orders_checkouts`, chống trùng theo `Idempotency-Key`), đơn, dòng
//! đơn, thuế của dòng, lịch sử trạng thái, khoản thanh toán, hoãn hoàn tất. Mọi index có `tenant_id`
//! đứng đầu (D-185). Tiền: cột `<tên>_minor` (BIGINT) cùng một cột `currency` của đơn — một đơn
//! chỉ có một loại tiền (D-20261118-step45-checkout).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0006_orders"
    }
}

const TABLES: [&str; 7] = [
    "orders_completion_holds",
    "orders_payments",
    "orders_status_history",
    "orders_line_taxes",
    "orders_lines",
    "orders_orders",
    "orders_checkouts",
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        checkouts(manager).await?;
        orders(manager).await?;
        lines(manager).await?;
        history_and_payments(manager).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

async fn checkouts(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table("orders_checkouts")
                // = ID của đơn sẽ tạo (một người bán) — mã tham chiếu giữ chỗ `orders:<id>`.
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                // `user:<id>` hay `cart:<id>` (vãng lai).
                .col(schema::string_len("owner_key", 80))
                // `Idempotency-Key` của người mua.
                .col(schema::string_len("reference", 200))
                .col(schema::string_len("fingerprint", 64))
                .col(schema::uuid("cart_id"))
                // `reserving`, `placed`, `failed`.
                .col(schema::string_len("state", 16))
                .col(schema::string_len_null("failure_code", 100))
                // JSON: chỗ đã giữ (`inventory`, `promotions`).
                .col(schema::text("holds"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .col(schema::timestamp_with_time_zone("updated_at"))
                .to_owned(),
        )
        .await?;
    let t = "orders_checkouts";
    index(
        manager,
        t,
        "orders_checkouts_reference_unique",
        &["tenant_id", "owner_key", "reference"],
        true,
    )
    .await?;
    index(manager, t, "orders_checkouts_state_idx", &["tenant_id", "state", "created_at"], false)
        .await
}

async fn orders(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let mut table = Table::create();
    table
        .table("orders_orders")
        .col(schema::pk_uuid("id"))
        .col(schema::uuid("tenant_id"))
        // Nút bán (cửa hàng / người bán).
        .col(schema::uuid("org_id"))
        .col(schema::uuid_null("checkout_id"))
        .col(schema::string_len("number", 40))
        // `web`, `pos` hay mã kênh ngoài.
        .col(schema::string_len("channel", 32))
        .col(schema::string_len_null("external_ref", 100))
        .col(schema::uuid_null("buyer_user_id"))
        // HMAC theo khách của số điện thoại chuẩn hoá (chống bom hàng, gắn đơn vãng lai).
        .col(schema::string_len_null("buyer_key", 64))
        // JSON: tên, số điện thoại, email; người nhận hộ.
        .col(schema::text("contact"))
        // JSON: địa chỉ giao (bản chụp).
        .col(schema::text_null("ship_to"))
        // JSON: cách giao đã chọn (bản chụp: phương thức, `option_key`, hãng, phí, ngày dự kiến).
        .col(schema::text_null("shipping_option"))
        .col(schema::uuid_null("pickup_org_id"))
        .col(schema::string_len("status", 32))
        .col(schema::string_len("payment_status", 32))
        .col(schema::string_len("fulfillment_status", 32))
        .col(schema::string_len("payment_method", 32))
        .col(schema::string_len("funds_flow", 32))
        .col(schema::string_len("currency", 3));
    for money in [
        "subtotal",
        "discount",
        "shipping_fee",
        "shipping_discount",
        "tax",
        "total",
        "paid",
        "refunded",
    ] {
        table.col(schema::big_integer(format!("{money}_minor")));
    }
    table
        // = `expires_at` của thanh toán ở `ledger` (D-218).
        .col(schema::timestamp_with_time_zone_null("payment_due_at"))
        .col(schema::text_null("attribution"))
        // JSON: kết quả khuyến mại lúc đặt (ảnh chụp cho đổi trả, hoá đơn).
        .col(schema::text_null("promotions"))
        .col(schema::boolean("stock_shortage"))
        .col(schema::text_null("invoice_request"))
        .col(schema::string_len_null("terms_version", 40))
        .col(schema::string_len_null("note", 1000))
        .col(schema::string_len("locale", 16))
        .col(schema::timestamp_with_time_zone("placed_at"))
        .col(schema::timestamp_with_time_zone_null("confirmed_at"))
        .col(schema::timestamp_with_time_zone_null("delivered_at"))
        .col(schema::timestamp_with_time_zone_null("completed_at"))
        .col(schema::timestamp_with_time_zone_null("cancelled_at"))
        .col(schema::string_len_null("cancel_reason", 200))
        // Tăng mỗi lần đổi (ETag / `If-Match`).
        .col(schema::integer("version"))
        .col(schema::timestamp_with_time_zone("created_at"))
        .col(schema::timestamp_with_time_zone("updated_at"));
    manager.create_table(table.to_owned()).await?;
    let t = "orders_orders";
    index(manager, t, "orders_orders_number_unique", &["tenant_id", "number"], true).await?;
    index(
        manager,
        t,
        "orders_orders_external_unique",
        &["tenant_id", "channel", "external_ref"],
        true,
    )
    .await?;
    index(manager, t, "orders_orders_org_idx", &["tenant_id", "org_id", "placed_at", "id"], false)
        .await?;
    index(manager, t, "orders_orders_buyer_idx", &["tenant_id", "buyer_user_id", "id"], false)
        .await?;
    index(manager, t, "orders_orders_due_idx", &["tenant_id", "status", "payment_due_at"], false)
        .await?;
    index(
        manager,
        t,
        "orders_orders_delivered_idx",
        &["tenant_id", "status", "delivered_at"],
        false,
    )
    .await?;
    index(manager, t, "orders_orders_buyer_key_idx", &["tenant_id", "buyer_key"], false).await
}

async fn lines(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let mut table = Table::create();
    table
        .table("orders_lines")
        .col(schema::pk_uuid("id"))
        .col(schema::uuid("tenant_id"))
        .col(schema::uuid("order_id"))
        // Khoá dòng gửi cho `inventory` / `promotions` (ID biến thể, hay `gift:<…>`).
        .col(schema::string_len("line_key", 120))
        .col(schema::uuid_null("parent_line_id"))
        .col(schema::uuid("variant_id"))
        .col(schema::uuid("product_id"))
        .col(schema::uuid_null("category_id"))
        .col(schema::uuid("seller_org_id"))
        // `physical`, `digital`, `service`, `bundle`.
        .col(schema::string_len("kind", 16))
        .col(schema::string_len("sku", 100))
        .col(schema::string_len("name", 300))
        // JSON: bản chụp biến thể (nhãn tuỳ chọn, ảnh, bảo hành, cân nặng…).
        .col(schema::text("snapshot"))
        .col(schema::string_len("sale_mode", 16))
        .col(schema::date_null("preorder_available_on"))
        .col(schema::boolean("requires_shipping"))
        .col(schema::boolean("tax_inclusive"))
        .col(schema::big_integer("quantity"));
    for money in ["unit_price", "subtotal", "discount", "net", "tax", "total"] {
        table.col(schema::big_integer(format!("{money}_minor")));
    }
    table
        .col(schema::big_integer_null("compare_at_minor"))
        // JSON: khoản giảm kèm lý do, ai trả (`funding`) và nguồn (D-222).
        .col(schema::text("discounts"))
        .col(schema::uuid_null("gift_of"))
        .col(schema::uuid_null("warehouse_org_id"))
        .col(schema::big_integer("returned_quantity"))
        .col(schema::timestamp_with_time_zone("created_at"));
    manager.create_table(table.to_owned()).await?;
    let t = "orders_lines";
    index(manager, t, "orders_lines_order_idx", &["tenant_id", "order_id"], false).await?;
    index(manager, t, "orders_lines_variant_idx", &["tenant_id", "variant_id", "order_id"], false)
        .await?;
    index(manager, t, "orders_lines_product_idx", &["tenant_id", "product_id"], false).await?;
    manager
        .create_table(
            Table::create()
                .table("orders_line_taxes")
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("line_id"))
                // Mã loại thuế của engine thuế (vd `vat`).
                .col(schema::string_len("tax_code", 40))
                // Chuỗi thập phân như `TaxEngine` trả (vd `"8"`).
                .col(schema::string_len("rate", 20))
                .col(schema::boolean("exempt"))
                .col(schema::big_integer("base_minor"))
                .col(schema::big_integer("amount_minor"))
                .primary_key(Index::create().col("tenant_id").col("line_id").col("tax_code"))
                .to_owned(),
        )
        .await
}

async fn history_and_payments(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table("orders_status_history")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("order_id"))
                .col(schema::string_len_null("from_status", 32))
                .col(schema::string_len("to_status", 32))
                // `buyer`, `staff`, `system`, `payment`, `shipping`, `channel`.
                .col(schema::string_len("cause", 16))
                .col(schema::uuid_null("actor_id"))
                .col(schema::string_len_null("reason", 200))
                .col(schema::timestamp_with_time_zone("at"))
                .to_owned(),
        )
        .await?;
    index(
        manager,
        "orders_status_history",
        "orders_status_history_order_idx",
        &["tenant_id", "order_id", "id"],
        false,
    )
    .await?;
    manager
        .create_table(
            Table::create()
                .table("orders_payments")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("order_id"))
                // `order`, `advance`, `balance` (D-232).
                .col(schema::string_len("purpose", 16))
                // `card`, `wallet`, `vietqr`, `bank_transfer`, `cod`, `cash`, `card_terminal`…
                .col(schema::string_len("method", 32))
                .col(schema::string_len_null("gateway", 64))
                // ID thanh toán ở `ledger`.
                .col(schema::uuid_null("ledger_payment_id"))
                .col(schema::string_len_null("external_id", 200))
                // Mã gửi đối tác / `ledger`: `orders:<order_id>:<purpose>:<lần>`.
                .col(schema::string_len("reference", 200))
                .col(schema::big_integer("amount_minor"))
                .col(schema::string_len("currency", 3))
                .col(schema::big_integer_null("tendered_minor"))
                // `pending`, `succeeded`, `failed`, `expired`, `refunded_partial`, `refunded`.
                .col(schema::string_len("status", 24))
                // JSON: việc người mua phải làm (`redirect_url`, `qr_payload`, `transfer_code`).
                .col(schema::text_null("action"))
                .col(schema::timestamp_with_time_zone_null("expires_at"))
                .col(schema::timestamp_with_time_zone_null("succeeded_at"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .col(schema::timestamp_with_time_zone("updated_at"))
                .to_owned(),
        )
        .await?;
    let t = "orders_payments";
    index(manager, t, "orders_payments_reference_unique", &["tenant_id", "reference"], true)
        .await?;
    index(manager, t, "orders_payments_order_idx", &["tenant_id", "order_id"], false).await?;
    index(manager, t, "orders_payments_ledger_idx", &["tenant_id", "ledger_payment_id"], false)
        .await?;
    manager
        .create_table(
            Table::create()
                .table("orders_completion_holds")
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("order_id"))
                .col(schema::string_len("reference", 200))
                .col(schema::timestamp_with_time_zone("created_at"))
                .primary_key(Index::create().col("tenant_id").col("order_id").col("reference"))
                .to_owned(),
        )
        .await
}
