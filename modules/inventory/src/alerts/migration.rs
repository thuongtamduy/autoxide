//! Bảng đăng ký báo khi có hàng lại (bước 42 mục 9, D-201).

use autoxide_kernel::migration::prelude::*;

use crate::migration::{drop_all, index, table};

pub(crate) struct CreateRestockAlerts;

impl MigrationName for CreateRestockAlerts {
    fn name(&self) -> &str {
        "m0012_create_restock_alerts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateRestockAlerts {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut alerts = table("inventory_restock_alerts");
        alerts
            .col(schema::uuid("variant_id"))
            .col(schema::uuid("store_key"))
            .col(schema::string_len("contact_kind", 10))
            .col(schema::string_len("contact", 320))
            .col(schema::uuid_null("user_id"))
            .col(schema::string_len("status", 20))
            .col(schema::string_len_null("token_hash", 64))
            .col(schema::text("consent"))
            .col(schema::string_len_null("locale", 10))
            .col(schema::boolean("armed"))
            .col(schema::big_integer("sent_count"))
            .col(schema::timestamp_with_time_zone_null("confirmed_at"))
            .col(schema::timestamp_with_time_zone("created_at"))
            .col(schema::timestamp_with_time_zone("updated_at"));
        manager.create_table(alerts).await?;
        let name = "inventory_restock_alerts";
        index(
            manager,
            name,
            "inventory_restock_alerts_key",
            &["tenant_id", "variant_id", "store_key", "contact_kind", "contact"],
            true,
        )
        .await?;
        index(
            manager,
            name,
            "inventory_restock_alerts_user_idx",
            &["tenant_id", "user_id", "variant_id"],
            false,
        )
        .await?;
        index(
            manager,
            name,
            "inventory_restock_alerts_token_idx",
            &["tenant_id", "token_hash"],
            false,
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_all(manager, &["inventory_restock_alerts"]).await
    }
}
