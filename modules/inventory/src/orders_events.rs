//! Event của `orders` (bước 45) mà `inventory` nghe bằng `Subscription::optional` — chưa cài
//! `orders` thì không lỗi khởi động. Mỗi event gọi đúng hàm service mà `api::Inventory` gọi, nên
//! event đến trước hay sau lời gọi trực tiếp, hay đến lại nhiều lần, kết quả như nhau (D-054).
//!
//! | Event | Việc | Dữ liệu đọc trong `after` |
//! |---|---|---|
//! | `orders.order_confirmed` | `confirm(order_id)` | `order_id` |
//! | `orders.order_cancelled` | `release(order_id, Cancelled)` | `order_id` |
//! | `orders.preorder_cancelled` | `release_lines(order_id, reference = event id)` — trả riêng các dòng | `order_id`, `lines[{line_key, quantity}]` (phần huỷ lần đó) |
//! | `orders.fulfillment_delivered` | xuất (`delivery`) mỗi dòng tới `delivered_quantity` (tổng đã giao; event cũ: thêm `quantity`), `fulfilment_key = fulfillment:<fulfillment_id>` — một lần giao tới tay người mua | `order_id`, `fulfillment_id`, `lines[{line_key, variant_id, quantity, delivered_quantity, serials[]}]` |
//! | `orders.order_delivered` | xuất (`delivery`) mỗi dòng của `delivered_lines` tới `delivered_quantity` (event cũ không có: phần còn giữ của mọi dòng), `fulfilment_key = order:<order_id>:delivered` (UT-1); không xuất được thì log + `inventory_delivery_unissued_total` | `order_id`, `delivered_lines[{line_key, variant_id, quantity, delivered_quantity, serials[]}]` |
//! | `orders.line_serials_recorded` | xuất (`delivery`) phần đã giao của dòng mà trước đó thiếu serial, `fulfilment_key = serials:<event id>` | `order_id`, `line_id`, `line_key`, `delivered_quantity`, `serials[]` |
//! | `orders.return_received` | `restock`, `reference = return:<return_id>` | `return_id`, `order_id`, `warehouse_org_id`, `lines[{variant_id, quantity, sellable, lot_code?, serials[]}]` |

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::tenancy::OrgId;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::{FulfilRequest, ReleaseReason, RestockLine, RestockRequest};
use crate::delivery::{DeliveredLine, Want};
use crate::partial::CancelledLine;
use crate::service::InventoryService;

pub(crate) const ORDER_CONFIRMED: &str = "orders.order_confirmed";
pub(crate) const ORDER_CANCELLED: &str = "orders.order_cancelled";
pub(crate) const RETURN_RECEIVED: &str = "orders.return_received";
pub(crate) const PREORDER_CANCELLED: &str = "orders.preorder_cancelled";
pub(crate) const ORDER_DELIVERED: &str = "orders.order_delivered";
pub(crate) const FULFILLMENT_DELIVERED: &str = "orders.fulfillment_delivered";
pub(crate) const LINE_SERIALS_RECORDED: &str = "orders.line_serials_recorded";

pub(crate) fn subscriptions(service: &Arc<InventoryService>) -> Vec<Subscription> {
    let listener = Arc::new(OrderEvents(service.clone()));
    vec![
        Subscription::optional(ORDER_CONFIRMED, listener.clone()),
        Subscription::optional(ORDER_CANCELLED, listener.clone()),
        Subscription::optional(RETURN_RECEIVED, listener.clone()),
        Subscription::optional(PREORDER_CANCELLED, listener.clone()),
        // Hàng tới tay người mua thì rời kho: ghi dòng `sale` (UT-1).
        Subscription::optional(FULFILLMENT_DELIVERED, listener.clone()),
        Subscription::optional(ORDER_DELIVERED, listener.clone()),
        Subscription::optional(LINE_SERIALS_RECORDED, listener),
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

/// Một dòng của `orders.preorder_cancelled`.
#[derive(Debug, Deserialize)]
struct CancelledLineIn {
    line_key: String,
    quantity: i64,
}

/// Một dòng đã giao trong `orders.fulfillment_delivered` / `orders.order_delivered`.
#[derive(Debug, Deserialize)]
struct ArrivedLine {
    line_key: String,
    #[serde(default)]
    variant_id: Option<Uuid>,
    quantity: i64,
    /// Tổng đơn vị của dòng đã tới tay người mua (event từ vòng sửa 3 của UT-1). Event cũ không có: `quantity`.
    #[serde(default)]
    delivered_quantity: Option<i64>,
    #[serde(default)]
    serials: Vec<String>,
}

impl ArrivedLine {
    fn wanted(self) -> DeliveredLine {
        let want = match self.delivered_quantity {
            Some(total) => Want::UpTo(total),
            None => Want::More(self.quantity),
        };
        DeliveredLine::new(self.line_key, want, self.serials)
    }
}

/// Biến thể nêu trong các dòng đã giao (để biết đơn không có chỗ giữ có hàng theo dõi tồn không).
fn variants_of(lines: &[ArrivedLine]) -> Vec<Uuid> {
    lines.iter().filter_map(|line| line.variant_id).collect()
}

/// `after` của `orders.fulfillment_delivered`.
#[derive(Debug, Deserialize)]
struct Arrived {
    order_id: Uuid,
    fulfillment_id: Uuid,
    lines: Vec<ArrivedLine>,
}

/// Một dòng đơn trong `after.lines` của event đơn.
#[derive(Debug, Deserialize)]
struct OrderLine {
    line_key: String,
}

/// `after` của `orders.order_delivered`.
#[derive(Debug, Deserialize)]
struct OrderArrived {
    order_id: Uuid,
    /// Có từ bản sửa UT-1: mọi dòng của đơn, kèm tổng đã giao (`delivered_quantity`).
    delivered_lines: Option<Vec<ArrivedLine>>,
    #[serde(default)]
    lines: Vec<OrderLine>,
}

/// `after` của `orders.line_serials_recorded`.
#[derive(Debug, Deserialize)]
struct SerialsRecorded {
    order_id: Uuid,
    line_key: String,
    delivered_quantity: i64,
    serials: Vec<String>,
}

/// `after` của `orders.preorder_cancelled`.
#[derive(Debug, Deserialize)]
struct Cancelled {
    order_id: Uuid,
    lines: Vec<CancelledLineIn>,
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
            ORDER_DELIVERED => {
                let Ok(arrived) = serde_json::from_value::<OrderArrived>(after.clone()) else {
                    tracing::warn!(event_id = %event.id, "orders.order_delivered without the fields inventory reads; ignored");
                    return Ok(());
                };
                let variants =
                    arrived.delivered_lines.as_deref().map(variants_of).unwrap_or_default();
                let wanted: Vec<DeliveredLine> = match arrived.delivered_lines {
                    Some(lines) => lines.into_iter().map(ArrivedLine::wanted).collect(),
                    // Event cũ: phần còn giữ của mọi dòng.
                    None if arrived.lines.is_empty() => vec![DeliveredLine::all()],
                    None => arrived
                        .lines
                        .into_iter()
                        .map(|line| DeliveredLine::new(line.line_key, Want::Remainder, Vec::new()))
                        .collect(),
                };
                let key = format!("order:{}:delivered", arrived.order_id);
                self.issue(event, arrived.order_id, key, &wanted, &variants).await
            }
            FULFILLMENT_DELIVERED => {
                let Ok(arrived) = serde_json::from_value::<Arrived>(after.clone()) else {
                    tracing::warn!(event_id = %event.id, "orders.fulfillment_delivered without the fields inventory reads; ignored");
                    return Ok(());
                };
                let variants = variants_of(&arrived.lines);
                let wanted: Vec<DeliveredLine> =
                    arrived.lines.into_iter().map(ArrivedLine::wanted).collect();
                let key = format!("fulfillment:{}", arrived.fulfillment_id);
                self.issue(event, arrived.order_id, key, &wanted, &variants).await
            }
            LINE_SERIALS_RECORDED => {
                let Ok(recorded) = serde_json::from_value::<SerialsRecorded>(after.clone()) else {
                    tracing::warn!(event_id = %event.id, "orders.line_serials_recorded without the fields inventory reads; ignored");
                    return Ok(());
                };
                // Khoá theo event: nhận lại cùng event thì không xuất lại. Hai lần ghi khác nhau (kể cả ghi đồng
                // thời, hay cùng số serial mà phần đã giao khác) là hai event, không đụng khoá của nhau; `UpTo`
                // tự kẹp theo phần đã xuất nên lần sau không xuất thừa.
                let key = format!("serials:{}", event.id);
                let wanted = [DeliveredLine::new(
                    recorded.line_key,
                    Want::UpTo(recorded.delivered_quantity),
                    recorded.serials,
                )];
                self.issue(event, recorded.order_id, key, &wanted, &[]).await
            }
            PREORDER_CANCELLED => {
                let Ok(cancelled) = serde_json::from_value::<Cancelled>(after.clone()) else {
                    tracing::warn!(event_id = %event.id, "orders.preorder_cancelled without the fields inventory reads; ignored");
                    return Ok(());
                };
                let lines: Vec<CancelledLine> = cancelled
                    .lines
                    .into_iter()
                    .map(|line| CancelledLine { line_key: line.line_key, quantity: line.quantity })
                    .collect();
                let reference = format!("{PREORDER_CANCELLED}:{}", event.id);
                let outcome = self
                    .0
                    .release_lines(tenant, cancelled.order_id, &reference, &lines, &Actor::system())
                    .await
                    .map(|_| ());
                settle(event, outcome)
            }
            _ => Ok(()),
        }
    }
}

impl OrderEvents {
    /// Xuất kho theo event giao hàng (`delivery`). Hàng đã đi rồi, nên lần xuất không được cũng không thử lại
    /// được: ghi cảnh báo có mã ổn định (`code`) và số đo `inventory_delivery_unissued_total` để người vận hành
    /// đối soát (vd chỉnh tồn tay). Đơn không có chỗ giữ mà không có hàng theo dõi tồn (hàng số, dịch vụ) là xong.
    async fn issue(
        &self,
        event: &Delivered,
        order: Uuid,
        key: String,
        wanted: &[DeliveredLine],
        variants: &[Uuid],
    ) -> Result<(), Error> {
        if wanted.is_empty() {
            return Ok(());
        }
        let request = FulfilRequest::new(order, key, Vec::new());
        let outcome = self.0.fulfil_with(&event.tenant, &request, Some(wanted)).await.map(|_| ());
        let Err(error) = outcome else { return Ok(()) };
        if error.kind().status() >= 500 {
            return Err(error);
        }
        let reason = if error.code() == "inventory.reservation_not_found" {
            if !self.tracks_stock(event, variants).await? {
                return Ok(());
            }
            "no_reservation"
        } else {
            "rejected"
        };
        metrics::counter!("inventory_delivery_unissued_total", "reason" => reason).increment(1);
        tracing::warn!(
            event_id = %event.id,
            event = %event.name,
            order_id = %order,
            code = error.code(),
            reason,
            "delivered goods were not issued from stock; reconcile the stock level by hand"
        );
        Ok(())
    }

    /// Có biến thể nào trong `variants` theo dõi tồn không (event cũ không nêu biến thể: không biết, coi là không).
    async fn tracks_stock(&self, event: &Delivered, variants: &[Uuid]) -> Result<bool, Error> {
        if variants.is_empty() {
            return Ok(false);
        }
        match self.0.variants(&event.tenant, variants).await {
            Ok(found) => Ok(found.values().any(|variant| variant.track_inventory)),
            // Biến thể không còn ở `catalog`: không biết, cảnh báo cho chắc.
            Err(error) if error.kind().status() < 500 => Ok(true),
            Err(error) => Err(error),
        }
    }
}
