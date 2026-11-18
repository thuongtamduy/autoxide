//! Event của `orders` (bước 45) mà `inventory` nghe bằng `Subscription::optional` — chưa cài
//! `orders` thì không lỗi khởi động. Mỗi event gọi đúng hàm service mà `api::Inventory` gọi, nên
//! event đến trước hay sau lời gọi trực tiếp, hay đến lại nhiều lần, kết quả như nhau (D-054).
//!
//! | Event | Việc | Dữ liệu đọc trong `after` |
//! |---|---|---|
//! | `orders.order_confirmed` | `confirm(order_id)` | `order_id` |
//! | `orders.order_cancelled` | `release(order_id, Cancelled)` | `order_id` |
//! | `orders.return_received` | `restock`, `reference = return:<return_id>` | `return_id`, `order_id`, `warehouse_org_id`, `lines[{variant_id, quantity, sellable, lot_code?, serials[]}]` |

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::tenancy::OrgId;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::{ReleaseReason, RestockLine, RestockRequest};
use crate::service::InventoryService;

pub(crate) const ORDER_CONFIRMED: &str = "orders.order_confirmed";
pub(crate) const ORDER_CANCELLED: &str = "orders.order_cancelled";
pub(crate) const RETURN_RECEIVED: &str = "orders.return_received";

pub(crate) fn subscriptions(service: &Arc<InventoryService>) -> Vec<Subscription> {
    let listener = Arc::new(OrderEvents(service.clone()));
    vec![
        Subscription::optional(ORDER_CONFIRMED, listener.clone()),
        Subscription::optional(ORDER_CANCELLED, listener.clone()),
        Subscription::optional(RETURN_RECEIVED, listener),
    ]
}

struct OrderEvents(Arc<InventoryService>);

/// Một dòng hàng trả về trong `orders.return_received`.
#[derive(Debug, Deserialize)]
struct ReturnedLine {
    variant_id: Uuid,
    quantity: i64,
    #[serde(default = "sellable")]
    sellable: bool,
    #[serde(default)]
    lot_code: Option<String>,
    #[serde(default)]
    serials: Vec<String>,
}

const fn sellable() -> bool {
    true
}

/// `after` của `orders.return_received`.
#[derive(Debug, Deserialize)]
struct Returned {
    return_id: Uuid,
    order_id: Option<Uuid>,
    warehouse_org_id: Uuid,
    lines: Vec<ReturnedLine>,
}

fn order_id(after: &Value) -> Option<Uuid> {
    after.get("order_id").and_then(Value::as_str).and_then(|id| id.parse().ok())
}

/// Lỗi nghiệp vụ không sửa được bằng cách giao lại event: ghi log và thôi (giao lại mãi cũng vậy).
fn settle(event: &Delivered, outcome: Result<(), Error>) -> Result<(), Error> {
    match outcome {
        Err(error) if error.kind().status() < 500 => {
            tracing::warn!(event_id = %event.id, event = %event.name, code = error.code(), "order event not applied to stock");
            Ok(())
        }
        other => other,
    }
}

#[async_trait::async_trait]
impl EventHandler for OrderEvents {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let after = &event.payload["after"];
        let tenant = &event.tenant;
        match event.name.as_str() {
            ORDER_CONFIRMED => {
                let Some(order) = order_id(after) else { return Ok(()) };
                let outcome = self.0.confirm(tenant, order).await.map(|_| ());
                // Đơn không có hàng theo dõi tồn (hàng số, dịch vụ) thì không có chỗ giữ.
                settle(event, outcome)
            }
            ORDER_CANCELLED => {
                let Some(order) = order_id(after) else { return Ok(()) };
                let outcome = self
                    .0
                    .release(tenant, order, ReleaseReason::Cancelled, &Actor::system())
                    .await
                    .map(|_| ());
                settle(event, outcome)
            }
            RETURN_RECEIVED => {
                let Ok(returned) = serde_json::from_value::<Returned>(after.clone()) else {
                    tracing::warn!(event_id = %event.id, "orders.return_received without the fields inventory reads; ignored");
                    return Ok(());
                };
                let lines = returned
                    .lines
                    .into_iter()
                    .map(|line| {
                        let mut restock =
                            RestockLine::new(line.variant_id, line.quantity, line.sellable);
                        restock.lot_code = line.lot_code;
                        restock.serials = line.serials;
                        restock
                    })
                    .collect();
                let request = RestockRequest::new(
                    format!("return:{}", returned.return_id),
                    returned.order_id,
                    OrgId::from_uuid(returned.warehouse_org_id),
                    lines,
                );
                let outcome = self.0.restock(tenant, &request).await;
                settle(event, outcome)
            }
            _ => Ok(()),
        }
    }
}
