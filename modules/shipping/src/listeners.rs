//! Nghe event của `orders` (bước 45) bằng `Subscription::optional`: `shipping` không phụ thuộc
//! `orders`, chỉ biết tên event dạng chuỗi và payload `after.shipment` là `ShipmentRequest`. Nhận
//! lại event không tạo vận đơn thứ hai (khoá `order_id` + `request_key`).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::tenancy::ScopeExt;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::api::{Direction, ShipmentRequest, events as names};
use crate::entity::shipment;
use crate::jobs::CancelShipment;
use crate::service::ShippingService;
use crate::shipments::status_of;
use crate::status;

pub(crate) fn subscriptions(service: &Arc<ShippingService>) -> Vec<Subscription> {
    vec![
        Subscription::optional(
            names::ORDERS_FULFILLMENT_REQUESTED,
            Arc::new(Requested { service: service.clone(), direction: Direction::Outbound }),
        ),
        Subscription::optional(
            names::ORDERS_RETURN_PICKUP_REQUESTED,
            Arc::new(Requested { service: service.clone(), direction: Direction::Return }),
        ),
        Subscription::optional(
            names::ORDERS_ORDER_CANCELLED,
            Arc::new(OrderCancelled(service.clone())),
        ),
    ]
}

/// `orders.fulfillment_requested` / `orders.return_pickup_requested` → vận đơn `pending`.
pub(crate) struct Requested {
    pub service: Arc<ShippingService>,
    pub direction: Direction,
}

#[async_trait::async_trait]
impl EventHandler for Requested {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Ok(mut request) =
            serde_json::from_value::<ShipmentRequest>(event.payload["after"]["shipment"].clone())
        else {
            tracing::error!(event = %event.name, "a shipment request event has no valid `after.shipment`; ignored");
            return Ok(());
        };
        request.direction = self.direction;
        match self.service.receive_request(&event.tenant, Actor::system(), &request).await {
            Ok(_) => Ok(()),
            // Nội dung sai hay khoá trùng khác nội dung: chạy lại không chữa được — ghi lỗi, bỏ qua.
            Err(error)
                if matches!(
                    error.code(),
                    "validation_failed" | "shipping.request_conflict" | "shipping.method_not_found"
                ) =>
            {
                tracing::error!(order_id = %request.order_id, code = error.code(), "shipment request rejected");
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

/// `orders.order_cancelled` → huỷ các vận đơn chưa lấy hàng của đơn (gọi hãng qua job).
pub(crate) struct OrderCancelled(pub Arc<ShippingService>);

#[async_trait::async_trait]
impl EventHandler for OrderCancelled {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let order = event.payload["after"]["order_id"]
            .as_str()
            .or_else(|| event.payload["after"]["id"].as_str())
            .and_then(|id| id.parse::<Uuid>().ok());
        let Some(order) = order else { return Ok(()) };
        let tenant = &event.tenant;
        let transaction = tenant.database().begin_write().await?;
        let rows = shipment::Entity::find()
            .filter(shipment::Column::OrderId.eq(order))
            .scoped(tenant)
            .all(&transaction)
            .await?;
        for row in rows {
            let current = status_of(&row);
            if status::picked_up(current)
                || !status::allowed(current, crate::api::ShipmentStatus::Cancelled)
            {
                continue;
            }
            self.0
                .jobs
                .enqueue(
                    &transaction,
                    tenant,
                    &CancelShipment { shipment_id: row.id },
                    EnqueueOptions::default().unique_key(format!("cancel:{}", row.id)),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}
