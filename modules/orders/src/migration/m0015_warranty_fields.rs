//! Ô người mua nhập lúc đặt hàng (`orders_orders.custom_fields`, D-235), thông tin trả góp của khoản
//! thu (`orders_payments.installment`), số serial ghi khi chuẩn bị hàng (`orders_line_serials`), bảo
//! hành (`orders_warranties`) và phiếu bảo hành (`orders_warranty_claims`). Mọi index có `tenant_id`
//! đứng đầu (D-185).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0015_warranty_fields"
    }
}

const TABLES: [&str; 3] = ["orders_warranty_claims", "orders_warranties", "orders_line_serials"];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite chỉ thêm được một cột mỗi câu `ALTER TABLE`.
        manager
            .alter_table(
                Table::alter().table("orders_orders").add_column(schema::text_null("custom_fields")).to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter().table("orders_payments").add_column(schema::text_null("installment")).to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_line_serials")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    .col(schema::uuid("line_id"))
                    .col(schema::string_len("serial", 100))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .primary_key(Index::create().col("tenant_id").col("line_id").col("serial"))
                    .to_owned(),
            )
            .await?;
        index(manager, "orders_line_serials", "orders_line_serials_order_idx", &["tenant_id", "order_id"], false)
            .await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_warranties")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("org_id"))
                    .col(schema::uuid("order_id"))
                    .col(schema::uuid("order_line_id"))
                    .col(schema::uuid("product_id"))
                    .col(schema::uuid("variant_id"))
                    .col(schema::string_len_null("serial", 100))
                    // Serial, hay `-` khi bảo hành theo dòng đơn (unique không dùng NULL).
                    .col(schema::string_len("serial_key", 100))
                    .col(schema::uuid_null("buyer_user_id"))
                    .col(schema::string_len_null("buyer_key", 64))
                    .col(schema::timestamp_with_time_zone("starts_at"))
                    .col(schema::timestamp_with_time_zone("ends_at"))
                    // `active`, `expired`, `void`.
                    .col(schema::string_len("status", 16))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_warranties";
        index(manager, t, "orders_warranties_line_key", &["tenant_id", "order_line_id", "serial_key"], true)
            .await?;
        index(manager, t, "orders_warranties_serial_idx", &["tenant_id", "serial"], false).await?;
        index(manager, t, "orders_warranties_buyer_idx", &["tenant_id", "buyer_user_id"], false).await?;
        index(manager, t, "orders_warranties_key_idx", &["tenant_id", "buyer_key"], false).await?;
        index(manager, t, "orders_warranties_due_idx", &["tenant_id", "status", "ends_at"], false).await?;
        manager
            .create_table(
                Table::create()
                    .table("orders_warranty_claims")
                    .col(schema::pk_uuid("id"))
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("warranty_id"))
                    // Nơi nhận (cửa hàng, trung tâm bảo hành).
                    .col(schema::uuid("org_id"))
                    .col(schema::string_len("number", 40))
                    // `received`, `inspecting`, `repairing`, `replacing`, `ready`, `returned`, `rejected`.
                    .col(schema::string_len("status", 16))
                    .col(schema::text("issue"))
                    .col(schema::text_null("resolution"))
                    .col(schema::timestamp_with_time_zone("due_at"))
                    .col(schema::uuid("created_by"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_warranty_claims";
        index(manager, t, "orders_warranty_claims_number_key", &["tenant_id", "number"], true).await?;
        index(manager, t, "orders_warranty_claims_org_idx", &["tenant_id", "org_id", "status", "id"], false).await?;
        index(manager, t, "orders_warranty_claims_warranty_idx", &["tenant_id", "warranty_id"], false).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in TABLES {
            manager.drop_table(Table::drop().table(table).to_owned()).await?;
        }
        manager
            .alter_table(Table::alter().table("orders_payments").drop_column("installment").to_owned())
            .await?;
        manager
            .alter_table(Table::alter().table("orders_orders").drop_column("custom_fields").to_owned())
            .await
    }
}
