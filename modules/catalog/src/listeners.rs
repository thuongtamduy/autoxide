//! Event `catalog` nghe. Bên nhận chịu được nhận lại một event (D-054): mọi việc ở đây là ghi đè
//! hay đặt lại, không cộng dồn.
//!
//! - `privacy::USER_DELETED` → bỏ `created_by` của sản phẩm, bảng giá, dòng giá người đó tạo.
//! - `inventory.cost_changed` (module `inventory`, bước 42, có thể chưa cài — nhận bằng
//!   `Subscription::optional`) → giá vốn trong `catalog_variant_costs` cho sàn giá (D-222). Payload
//!   mong đợi: `{"after": {"org_id", "variant_id", "unit_cost": {"amount": "<số nguyên>",
//!   "currency": "VND"}}}` — xem `api::events::INVENTORY_COST_CHANGED`.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, Subscription};
use autoxide_kernel::money::Money;
use autoxide_kernel::privacy::deleted_user;
use autoxide_kernel::tenancy::ScopeExt;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::api::events::INVENTORY_COST_CHANGED;
use crate::entity::{price, price_list, product};
use crate::service::CatalogService;

/// Nút tổ chức đổi chỗ hay đổi trạng thái (module `organizations`): website nào thấy sản phẩm nào
/// đổi theo cây, nên dựng lại storefront của khách.
const UNIT_MOVED: &str = "organizations.unit_moved";
const UNIT_STATUS_CHANGED: &str = "organizations.unit_status_changed";

pub(crate) fn subscriptions(service: &Arc<CatalogService>) -> Vec<Subscription> {
    vec![
        Subscription::new(autoxide_kernel::privacy::USER_DELETED, service.clone()),
        Subscription::optional(INVENTORY_COST_CHANGED, service.clone()),
        Subscription::optional(UNIT_MOVED, service.clone()),
        Subscription::optional(
            crate::api::events::PROMOTIONS_AUTOMATIC_PRICES_CHANGED,
            service.clone(),
        ),
        Subscription::optional(UNIT_STATUS_CHANGED, service.clone()),
    ]
}

#[async_trait::async_trait]
impl EventHandler for CatalogService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if let Some(user_id) = deleted_user(event) {
            return self.forget_user(event, user_id).await;
        }
        if event.name == INVENTORY_COST_CHANGED {
            let after = &event.payload["after"];
            let parsed = (|| {
                let org: Uuid = after.get("org_id")?.as_str()?.parse().ok()?;
                let variant: Uuid = after.get("variant_id")?.as_str()?.parse().ok()?;
                let cost: Money = serde_json::from_value(after.get("unit_cost")?.clone()).ok()?;
                Some((org, variant, cost))
            })();
            let Some((org, variant, cost)) = parsed else {
                tracing::warn!(event_id = %event.id, "inventory.cost_changed without org_id, variant_id or unit_cost; ignored");
                return Ok(());
            };
            return self.record_cost(&event.tenant, org, variant, cost).await;
        }
        if event.name == crate::api::events::PROMOTIONS_AUTOMATIC_PRICES_CHANGED {
            let variants: Vec<Uuid> = event.payload["after"]["variant_ids"]
                .as_array()
                .map(|ids| ids.iter().filter_map(|id| id.as_str()?.parse().ok()).collect())
                .unwrap_or_default();
            if variants.is_empty() {
                tracing::warn!(event_id = %event.id, "promotions.automatic_prices_changed without variant_ids; ignored");
                return Ok(());
            }
            // Job tính lại từ đầu: nhận lại event chỉ dựng lại thêm một lần.
            let write = event.tenant.database().write();
            return self.queue_variants(write, &event.tenant, variants).await;
        }
        if event.name == UNIT_MOVED || event.name == UNIT_STATUS_CHANGED {
            // Job tính lại từ đầu: nhận lại event chỉ dựng lại thêm một lần.
            let write = event.tenant.database().write();
            return self.queue_all(write, &event.tenant).await;
        }
        Ok(())
    }
}

impl CatalogService {
    async fn forget_user(&self, event: &Delivered, user_id: Uuid) -> Result<(), Error> {
        let tenant = &event.tenant;
        let write = tenant.database().write();
        product::Entity::update_many()
            .col_expr(product::Column::CreatedBy, Expr::value(None::<Uuid>))
            .filter(product::Column::CreatedBy.eq(user_id))
            .scoped(tenant)
            .exec(write)
            .await?;
        price_list::Entity::update_many()
            .col_expr(price_list::Column::CreatedBy, Expr::value(None::<Uuid>))
            .filter(price_list::Column::CreatedBy.eq(user_id))
            .scoped(tenant)
            .exec(write)
            .await?;
        price::Entity::update_many()
            .col_expr(price::Column::CreatedBy, Expr::value(None::<Uuid>))
            .filter(price::Column::CreatedBy.eq(user_id))
            .scoped(tenant)
            .exec(write)
            .await?;
        tracing::info!(user_id = %user_id, "catalog records of a deleted user anonymised");
        Ok(())
    }
}
