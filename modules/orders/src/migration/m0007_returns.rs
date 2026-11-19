//! Hoàn tiền và đổi trả: `orders_refunds` (mọi lượt hoàn của đơn — huỷ, trả hàng, tranh chấp, hoàn
//! tay; chống trùng theo `reference`) và `orders_returns` (yêu cầu trả hàng của người mua). Mọi index
//! có `tenant_id` đứng đầu (D-185); tiền là `<tên>_minor` + `currency` (D-20261119-step45-returns).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0007_returns"
    }
}

const TABLES: [&str; 2] = ["orders_returns", "orders_refunds"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        refunds(manager).await?;
        returns(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}

async fn refunds(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table("orders_refunds")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("order_id"))
                .col(schema::uuid_null("return_id"))
                // Khoản thanh toán (`orders_payments`) được hoàn.
                .col(schema::uuid("payment_id"))
                // Mã chống trùng của bên gọi (`orders:<id>:cancel:<khoản>`, `return:<id>`, mã của
                // nhân viên hay của tranh chấp).
                .col(schema::string_len("reference", 200))
                .col(schema::string_len("reference_fingerprint", 64))
                .col(schema::big_integer("amount_minor"))
                .col(schema::string_len("currency", 3))
                // `pending`, `succeeded`, `failed`, `manual` (đã trả ngoài cổng: tiền mặt, chuyển khoản).
                .col(schema::string_len("status", 16))
                // ID lượt hoàn ở `ledger` khi hoàn qua cổng.
                .col(schema::uuid_null("ledger_refund_id"))
                .col(schema::string_len("reason", 200))
                .col(schema::uuid_null("created_by"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .col(schema::timestamp_with_time_zone("updated_at"))
                .to_owned(),
        )
        .await?;
    let t = "orders_refunds";
    index(manager, t, "orders_refunds_reference_unique", &["tenant_id", "reference"], true).await?;
    index(manager, t, "orders_refunds_order_idx", &["tenant_id", "order_id"], false).await?;
    index(manager, t, "orders_refunds_ledger_idx", &["tenant_id", "ledger_refund_id"], false).await
}

async fn returns(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .create_table(
            Table::create()
                .table("orders_returns")
                .col(schema::pk_uuid("id"))
                .col(schema::uuid("tenant_id"))
                .col(schema::uuid("order_id"))
                // Nút bán của đơn (phạm vi quyền của nhân viên duyệt).
                .col(schema::uuid("org_id"))
                .col(schema::uuid_null("buyer_user_id"))
                .col(schema::string_len("number", 40))
                // `requested`, `approved`, `rejected`, `received`, `refunded`, `closed`.
                .col(schema::string_len("status", 16))
                // Mã của danh sách `orders.return_reason` (D-223).
                .col(schema::string_len("reason_code", 64))
                .col(schema::text_null("buyer_note"))
                // `refund`, `exchange`.
                .col(schema::string_len("resolution", 16))
                // `pickup`, `drop_off`, `in_store` (D-201).
                .col(schema::string_len("return_method", 16))
                // JSON: danh sách UUID ảnh (tối đa 6, D-202).
                .col(schema::text("media_ids"))
                // JSON: `[{line_id, quantity, received, condition}]`.
                .col(schema::text("lines"))
                // Tiền hoàn tính lúc nhận hàng.
                .col(schema::big_integer_null("refund_minor"))
                // JSON: cách tính tiền hoàn (hàng, phí giao, khoản giảm thu lại, giá trị quà trừ).
                .col(schema::text_null("breakdown"))
                .col(schema::string_len("currency", 3))
                // Mã của danh sách `orders.return_rejected` khi từ chối.
                .col(schema::string_len_null("reject_reason", 64))
                .col(schema::uuid_null("warehouse_org_id"))
                .col(schema::timestamp_with_time_zone("requested_at"))
                .col(schema::timestamp_with_time_zone("deadline_at"))
                .col(schema::uuid_null("decided_by"))
                .col(schema::timestamp_with_time_zone_null("decided_at"))
                .col(schema::timestamp_with_time_zone_null("received_at"))
                .col(schema::timestamp_with_time_zone("created_at"))
                .col(schema::timestamp_with_time_zone("updated_at"))
                .to_owned(),
        )
        .await?;
    let t = "orders_returns";
    index(manager, t, "orders_returns_number_unique", &["tenant_id", "number"], true).await?;
    index(manager, t, "orders_returns_org_idx", &["tenant_id", "org_id", "status", "id"], false)
        .await?;
    index(manager, t, "orders_returns_order_idx", &["tenant_id", "order_id"], false).await
}
