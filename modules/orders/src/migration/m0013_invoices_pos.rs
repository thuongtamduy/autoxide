//! Hoá đơn điện tử của đơn (`orders_invoice_links`, D-215) và bán tại quầy (`orders_pos_shifts`,
//! `orders_pos_sales`, D-227, D-340). Mọi index có `tenant_id` đứng đầu (D-185).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0013_invoices_pos"
    }
}

const TABLES: [&str; 3] = ["orders_pos_sales", "orders_pos_shifts", "orders_invoice_links"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("orders_invoice_links")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    // Lần trả hàng của hoá đơn điều chỉnh.
                    .col(schema::uuid_null("return_id"))
                    // `sales`, `cash_register`, `adjustment`.
                    .col(schema::string_len("kind", 16))
                    // Chứng từ của `invoices` (chỉ giữ ID).
                    .col(schema::uuid("document_id"))
                    .col(schema::string_len("status", 16))
                    // Mã tham chiếu gửi `invoices` (`orders:<id>:invoice`…).
                    .col(schema::string_len("reference", 200))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_invoice_links";
        index(manager, t, "orders_invoice_links_reference_key", &["tenant_id", "reference"], true)
            .await?;
        index(manager, t, "orders_invoice_links_order_idx", &["tenant_id", "order_id"], false)
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_pos_shifts")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    // Cửa hàng.
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("register_code", 40))
                    // `<org>:<quầy>` khi ca đang mở, NULL khi đã đóng: unique — một ca mở mỗi quầy.
                    .col(schema::string_len_null("open_key", 100))
                    .col(schema::uuid("opened_by"))
                    .col(schema::timestamp_with_time_zone("opened_at"))
                    .col(schema::string_len("currency", 3))
                    .col(schema::big_integer("opening_cash_minor"))
                    .col(schema::uuid_null("closed_by"))
                    .col(schema::timestamp_with_time_zone_null("closed_at"))
                    .col(schema::big_integer_null("expected_cash_minor"))
                    .col(schema::big_integer_null("counted_cash_minor"))
                    .col(schema::big_integer_null("variance_minor"))
                    .col(schema::string_len_null("variance_reason", 64))
                    .col(schema::string_len_null("variance_note", 1000))
                    .col(schema::uuid_null("approved_by"))
                    .col(schema::timestamp_with_time_zone_null("approved_at"))
                    // `open`, `pending_approval`, `closed`.
                    .col(schema::string_len("status", 20))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_pos_shifts";
        index(manager, t, "orders_pos_shifts_open_key", &["tenant_id", "open_key"], true).await?;
        index(
            manager,
            t,
            "orders_pos_shifts_org_idx",
            &["tenant_id", "org_id", "opened_at"],
            false,
        )
        .await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_pos_sales")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("shift_id"))
                    // Mã của máy quầy (chống trùng khi máy gửi lại, kể cả lúc ngoại tuyến).
                    .col(schema::string_len("reference", 200))
                    .col(schema::string_len("fingerprint", 64))
                    .col(schema::uuid("order_id"))
                    // Tiền mặt thực vào két (đã trừ tiền thối) — để tính tiền két dự kiến.
                    .col(schema::big_integer("cash_minor"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("shift_id").col("reference"))
                    .to_owned(),
            )
            .await?;
        index(
            manager,
            "orders_pos_sales",
            "orders_pos_sales_order_idx",
            &["tenant_id", "order_id"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        Ok(())
    }
}
