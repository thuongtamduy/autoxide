//! Xác nhận / trả chỗ giữ sau khi đơn đã tồn tại đi qua event của chính `orders` (đặc tả 45 "quy ước
//! một cách làm"): `inventory` tự nghe `orders.order_confirmed` / `orders.order_cancelled`;
//! `promotions` không nghe `orders` nên `orders` tự nghe event của mình rồi gọi
//! `Promotions::confirm` / `release` — cả hai chịu được gọi lại. Lượt đặt treo ở `reserving` quá
//! `AUTOXIDE_ORDERS_CHECKOUT_STUCK_MINUTES` (máy chết giữa giữ chỗ và tạo đơn) được job
//! `orders.recover_checkouts` trả chỗ.

use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_inventory::api::ReleaseReason;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler, Subscription};
use autoxide_kernel::tenancy::TenantId;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::api::events as names;
use crate::checkout::{Holds, RESERVING};
use crate::entity::checkout;
use crate::jobs::BATCH;
use crate::service::OrdersService;

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    let listener = Arc::new(OwnEvents(service.clone()));
    vec![
        Subscription::new(names::ORDER_CONFIRMED, listener.clone()),
        Subscription::new(names::ORDER_CANCELLED, listener),
    ]
}

struct OwnEvents(Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for OwnEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(promotions) = &self.0.sources.promotions else { return Ok(()) };
        let Some(order) =
            event.payload["after"]["order_id"].as_str().and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        let outcome = match event.name.as_str() {
            names::ORDER_CONFIRMED => promotions.confirm(&event.tenant, order).await.map(|_| ()),
            names::ORDER_CANCELLED => promotions.release(&event.tenant, order).await.map(|_| ()),
            _ => Ok(()),
        };
        match outcome {
            // Đơn không có lượt khuyến mại nào được giữ, hay lỗi nghiệp vụ: giao lại không chữa được.
            Err(error) if error.kind().status() < 500 => {
                tracing::debug!(order_id = %order, code = error.code(), "promotion hold not changed");
                Ok(())
            }
            other => other,
        }
    }
}

impl OrdersService {
    /// Một lô lượt đặt treo ở `reserving` trong `database` (mọi khách): trả chỗ, đánh dấu `failed`.
    pub(crate) async fn recover_checkouts(&self, database: &Database) -> Result<u64, Error> {
        let before =
            crate::service::now() - SignedDuration::from_mins(self.config.checkout_stuck_minutes);
        let stuck: Vec<(Uuid, Uuid)> = checkout::Entity::find()
            .select_only()
            .column(checkout::Column::TenantId)
            .column(checkout::Column::Id)
            .filter(checkout::Column::State.eq(RESERVING))
            .filter(checkout::Column::CreatedAt.lt(before))
            .order_by_asc(checkout::Column::CreatedAt)
            .limit(BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut recovered = 0;
        for (tenant_id, id) in stuck {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            // Không biết đã giữ tới đâu: trả cả hai (trả khi chưa giữ là không làm gì).
            let holds = Holds { inventory: true, promotions: true, tenders: self.all_tenders() };
            self.release_holds(&tenant, id, &holds, ReleaseReason::CheckoutAborted).await;
            self.fail_checkout(&tenant, id, "orders.checkout_stuck").await?;
            recovered += 1;
        }
        Ok(recovered)
    }
}
