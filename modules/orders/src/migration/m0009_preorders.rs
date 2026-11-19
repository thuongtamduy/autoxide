//! Hàng đặt trước và giao nhiều lần (D-204, D-20261119-step45-pickup-preorder): `orders_fulfillments`
//! (mỗi lần giao một dòng, khoá yêu cầu gửi `shipping`), thêm cột cho dòng đơn (đã chia hàng, đã huỷ,
//! ngày giao dời tới) và cho đơn (lúc người mua đồng ý điều kiện đặt trước). Mỗi `ALTER TABLE` một
//! cột: SQLite không nhận nhiều thay đổi trong một lệnh.

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0009_preorders"
    }
}

/// (bảng, cột) thêm vào.
const ADDED: [(&str, &str); 4] = [
    ("orders_lines", "allocated_quantity"),
    ("orders_lines", "cancelled_quantity"),
    ("orders_lines", "preorder_delayed_to"),
    ("orders_orders", "preorder_accepted_at"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("orders_fulfillments")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    // Khoá gửi `shipping` (`fulfillment:<id>`): gửi lại không tạo vận đơn thứ hai.
                    .col(schema::string_len("request_key", 100))
                    // JSON: `[{line_id, quantity}]`.
                    .col(schema::text("lines"))
                    // `requested`, `shipping`, `delivered`, `failed`.
                    .col(schema::string_len("status", 16))
                    // Tiền COD hãng đã thu cho lần giao này (chống cộng hai lần khi event tới lại).
                    .col(schema::big_integer_null("cod_collected_minor"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_fulfillments";
        index(manager, t, "orders_fulfillments_key_unique", &["tenant_id", "request_key"], true)
            .await?;
        index(manager, t, "orders_fulfillments_order_idx", &["tenant_id", "order_id"], false)
            .await?;
        let columns = [
            schema::big_integer("allocated_quantity").default(0).to_owned(),
            schema::big_integer("cancelled_quantity").default(0).to_owned(),
            schema::date_null("preorder_delayed_to").to_owned(),
            schema::timestamp_with_time_zone_null("preorder_accepted_at").to_owned(),
        ];
        for ((table, _), mut column) in ADDED.into_iter().zip(columns) {
            manager
                .alter_table(Table::alter().table(table).add_column(&mut column).to_owned())
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for (table, column) in ADDED.into_iter().rev() {
            manager.alter_table(Table::alter().table(table).drop_column(column).to_owned()).await?;
        }
        manager.drop_table(Table::drop().table("orders_fulfillments").to_owned()).await
    }
}
