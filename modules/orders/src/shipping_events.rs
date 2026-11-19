//! Nghe event vận đơn của `shipping` (bước 44, `Subscription::optional`: chưa cài thì thôi) và đổi
//! trạng thái đơn qua máy trạng thái: đã lấy hàng → `shipping`, đã giao → `delivered`, giao hỏng /
//! hàng hoàn → `delivery_failed`; tiền COD hãng đã thu → đơn đã thu. Event tới hai lần hay sai thứ
//! tự (đã giao tới trước đã lấy hàng) không làm sai: chuyển tới trạng thái đang có là không làm gì,
//! chuyển không có trong bảng bị bỏ qua (ghi log).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_shipping::api::{Direction, ShipmentEvent, ShipmentStatus, events as names};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::entity::{order, payment};
use crate::lifecycle::{self, Cause, Transition};
use crate::payments::{self, Method};
use crate::service::OrdersService;

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    let listener = Arc::new(ShippingEvents(service.clone()));
    [
        names::SHIPMENT_STATUS_CHANGED,
        names::SHIPMENT_DELIVERED,
        names::SHIPMENT_DELIVERY_FAILED,
        names::SHIPMENT_RETURNED,
        names::COD_COLLECTED,
    ]
    .into_iter()
    .map(|name| Subscription::optional(name, listener.clone()))
    .collect()
}

struct ShippingEvents(Arc<OrdersService>);

/// Trạng thái đơn ứng với trạng thái vận đơn (chiều giao đi).
fn target(status: ShipmentStatus) -> Option<OrderStatus> {
    match status {
        ShipmentStatus::PickedUp
        | ShipmentStatus::InTransit
        | ShipmentStatus::OutForDelivery
        | ShipmentStatus::ReadyForPickup => Some(OrderStatus::Shipping),
        ShipmentStatus::Delivered | ShipmentStatus::PickedUpByBuyer => Some(OrderStatus::Delivered),
        ShipmentStatus::DeliveryFailed | ShipmentStatus::Returning | ShipmentStatus::Returned => {
            Some(OrderStatus::DeliveryFailed)
        }
        _ => None,
    }
}

#[async_trait::async_trait]
impl EventHandler for ShippingEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Ok(shipment) = serde_json::from_value::<ShipmentEvent>(event.payload["after"].clone())
        else {
            tracing::warn!(event_id = %event.id, event = %event.name, "shipment event without the fields orders reads; ignored");
            return Ok(());
        };
        if shipment.direction != Direction::Outbound {
            return Ok(());
        }
        let tenant = &event.tenant;
        if event.name == names::COD_COLLECTED {
            if let Some(amount) = shipment.cod_amount {
                return self.0.cod_collected(tenant, shipment.order_id, amount).await;
            }
            return Ok(());
        }
        let Some(to) = target(shipment.status) else { return Ok(()) };
        self.0.follow_shipment(tenant, shipment.order_id, to).await
    }
}

impl OrdersService {
    /// Đưa đơn tới `to` theo vận đơn; đi qua các bước trung gian của bảng (`confirmed` →
    /// `preparing` → `shipping` → `delivered`), ghi đủ lịch sử từng bước.
    pub(crate) async fn follow_shipment(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        to: OrderStatus,
    ) -> Result<(), Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let mut current = match lifecycle::lock(&transaction, tenant, order_id).await {
                    Ok(current) => current,
                    // Vận đơn không thuộc đơn nào của `orders` (tạo tay ở `shipping`).
                    Err(error) if error.code() == "orders.order_not_found" => return Ok(()),
                    Err(error) => return Err(error),
                };
                for step in path(&current.status, to) {
                    let transition = Transition::new(step, Cause::Shipping, Actor::system());
                    lifecycle::apply(
                        &transaction,
                        tenant,
                        outbox,
                        current.clone(),
                        &transition,
                        |_| {},
                    )
                    .await?;
                    current = lifecycle::lock(&transaction, tenant, order_id).await?;
                }
                transaction.commit().await?;
                Ok(())
            })
            .await
    }

    /// Hãng đã thu tiền COD của đơn: ghi đã thu (một lần).
    pub(crate) async fn cod_collected(
        &self,
        tenant: &TenantContext,
        order_id: Uuid,
        amount: Money,
    ) -> Result<(), Error> {
        let database = tenant.database();
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = match lifecycle::lock(&transaction, tenant, order_id).await {
                    Ok(current) => current,
                    Err(error) if error.code() == "orders.order_not_found" => return Ok(()),
                    Err(error) => return Err(error),
                };
                let Some(row) = payment::Entity::find()
                    .filter(payment::Column::OrderId.eq(order_id))
                    .filter(payment::Column::Status.eq(payments::PENDING))
                    .scoped(tenant)
                    .all(&transaction)
                    .await?
                    .into_iter()
                    .find(|row| Method::parse(&row.method) == Method::Cod)
                else {
                    return Ok(());
                };
                let now = crate::service::now();
                let mut paid: payment::ActiveModel = row.into();
                paid.status = Set(payments::SUCCEEDED.to_owned());
                paid.succeeded_at = Set(Some(now));
                paid.updated_at = Set(now);
                paid.update(&transaction).await?;
                let mut updated: order::ActiveModel = current.clone().into();
                updated.paid_minor = Set(current.paid_minor + amount.minor());
                updated.payment_status = Set(payments::PAID.to_owned());
                updated.updated_at = Set(now);
                updated.update(&transaction).await?;
                transaction.commit().await?;
                Ok(())
            })
            .await
    }
}

/// Các bước của bảng chuyển từ `from` tới `to` theo đường giao hàng; rỗng khi đã tới hay không đi
/// được (đơn đã huỷ / hoàn tất).
fn path(from: &str, to: OrderStatus) -> Vec<OrderStatus> {
    use OrderStatus::{Confirmed, Delivered, DeliveryFailed, Preparing, Shipping};
    let Some(from) = OrderStatus::parse(from) else { return Vec::new() };
    let road = [Confirmed, Preparing, Shipping, Delivered];
    match (from, to) {
        (DeliveryFailed, Shipping | Delivered) => {
            let mut steps = vec![Shipping];
            if to == Delivered {
                steps.push(Delivered);
            }
            steps
        }
        (Confirmed | Preparing | Shipping, DeliveryFailed) => {
            let mut steps: Vec<OrderStatus> = road
                .iter()
                .copied()
                .skip_while(|step| *step != from)
                .skip(1)
                .take_while(|step| *step != Delivered)
                .collect();
            if !steps.contains(&Shipping) && from != Shipping {
                steps.push(Shipping);
            }
            steps.retain(|step| *step != from);
            steps.push(DeliveryFailed);
            steps
        }
        _ => {
            let Some(start) = road.iter().position(|step| *step == from) else { return Vec::new() };
            let Some(end) = road.iter().position(|step| *step == to) else { return Vec::new() };
            if end <= start { Vec::new() } else { road[start + 1..=end].to_vec() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::path;
    use crate::api::OrderStatus::*;

    #[test]
    fn duong_di_cua_don_theo_van_don() {
        assert_eq!(path("confirmed", Delivered), vec![Preparing, Shipping, Delivered]);
        assert_eq!(path("preparing", Shipping), vec![Shipping]);
        assert_eq!(path("shipping", Shipping), vec![]);
        assert_eq!(path("delivered", Shipping), vec![]);
        assert_eq!(path("shipping", DeliveryFailed), vec![DeliveryFailed]);
        assert_eq!(path("preparing", DeliveryFailed), vec![Shipping, DeliveryFailed]);
        assert_eq!(path("delivery_failed", Delivered), vec![Shipping, Delivered]);
        assert_eq!(path("cancelled", Delivered), vec![]);
    }
}
