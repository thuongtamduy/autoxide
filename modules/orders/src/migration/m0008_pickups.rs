//! Nhận tại cửa hàng (D-201, B-c7): `orders_pickups` — một dòng cho mỗi đơn nhận tại cửa hàng đã sẵn
//! sàng (mã nhận, hạn giữ hàng, ai đã nhận). Mọi index có `tenant_id` đứng đầu (D-185).

use autoxide_kernel::migration::prelude::*;

use super::index;

pub(crate) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m0008_pickups"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table("orders_pickups")
                    .col(schema::uuid("tenant_id"))
                    .col(schema::uuid("order_id"))
                    // Cửa hàng giữ hàng chờ nhận.
                    .col(schema::uuid("pickup_org_id"))
                    // `ready`, `picked_up`, `expired`.
                    .col(schema::string_len("status", 16))
                    // Mã nhận 6 số người mua đọc ở quầy.
                    .col(schema::string_len("code", 6))
                    .col(schema::timestamp_with_time_zone("ready_at"))
                    .col(schema::timestamp_with_time_zone("hold_until"))
                    .col(schema::timestamp_with_time_zone_null("reminded_at"))
                    .col(schema::timestamp_with_time_zone_null("handed_over_at"))
                    // JSON: `{"by": "code"}` hay `{"by": "proxy", "name", "phone"}`.
                    .col(schema::text_null("handed_over_to"))
                    .col(schema::timestamp_with_time_zone("created_at"))
                    .col(schema::timestamp_with_time_zone("updated_at"))
                    .primary_key(Index::create().col("tenant_id").col("order_id"))
                    .to_owned(),
            )
            .await?;
        let t = "orders_pickups";
        index(
            manager,
            t,
            "orders_pickups_org_idx",
            &["tenant_id", "pickup_org_id", "status"],
            false,
        )
        .await?;
        index(manager, t, "orders_pickups_due_idx", &["tenant_id", "status", "hold_until"], false)
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table("orders_pickups").to_owned()).await
    }
}
