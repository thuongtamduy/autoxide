//! Event module `orders` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module; ghi CÙNG
//! transaction với thay đổi (luật kiến trúc 3). Tên event là hằng trong `api::events` để module khác
//! nghe đúng tên.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

use crate::api::events as names;

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
}

impl Change {
    pub(crate) fn after(after: Value) -> Self {
        Change { before: None, after: Some(after) }
    }
}

/// Một loại event: tên + dữ liệu `Change`.
macro_rules! event {
    ($kind:ident, $name:expr) => {
        #[derive(Debug, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $kind(pub Change);

        impl Event for $kind {
            const NAME: &'static str = $name;
        }
    };
}

event!(CartAbandoned, names::CART_ABANDONED);
event!(OrderPlaced, names::ORDER_PLACED);
event!(OrderConfirmed, names::ORDER_CONFIRMED);
event!(OrderStatusChanged, names::ORDER_STATUS_CHANGED);
event!(OrderCancelled, names::ORDER_CANCELLED);
event!(OrderDelivered, names::ORDER_DELIVERED);
event!(OrderCompleted, names::ORDER_COMPLETED);
event!(PaymentRecorded, names::PAYMENT_RECORDED);
event!(FulfillmentRequested, names::FULFILLMENT_REQUESTED);
event!(RefundCompleted, names::REFUND_COMPLETED);
event!(ReturnRequested, names::RETURN_REQUESTED);
event!(ReturnPickupRequested, names::RETURN_PICKUP_REQUESTED);
event!(ReturnReceived, names::RETURN_RECEIVED);
event!(ReturnStatusChanged, names::RETURN_STATUS_CHANGED);

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        EventInfo::new(names::ORDER_PLACED, "An order was placed (pending payment or confirmed)."),
        EventInfo::new(
            names::ORDER_CONFIRMED,
            "An order was confirmed (paid, cash on delivery accepted, point of sale or channel); \
             holds of stock and promotions are confirmed.",
        ),
        EventInfo::new(names::ORDER_STATUS_CHANGED, "The status of an order changed (any change)."),
        EventInfo::new(
            names::ORDER_CANCELLED,
            "An order was cancelled; holds of stock and promotions are released.",
        ),
        EventInfo::new(names::ORDER_DELIVERED, "An order was delivered to the buyer."),
        EventInfo::new(
            names::ORDER_COMPLETED,
            "An order was completed: the return window ended with nothing open.",
        ),
        EventInfo::new(
            names::FULFILLMENT_REQUESTED,
            "A shipment was requested for an order; `after.shipment` is the shipment request.",
        ),
        EventInfo::new(
            names::RETURN_PICKUP_REQUESTED,
            "A carrier pickup was requested for a return; `after.shipment` is the shipment request.",
        ),
        EventInfo::new(names::STOCK_SHORTAGE, "An order imported from a channel lacks stock."),
        EventInfo::new(names::PAYMENT_RECORDED, "A payment of an order was recorded."),
        EventInfo::new(names::REFUND_COMPLETED, "A refund of an order was completed."),
        EventInfo::new(names::RETURN_REQUESTED, "A buyer asked to return items of an order."),
        EventInfo::new(
            names::RETURN_RECEIVED,
            "Returned items arrived back at the warehouse (restocked or damaged).",
        ),
        EventInfo::new(
            names::RETURN_STATUS_CHANGED,
            "A return changed status (approved, rejected, received, refunded or closed).",
        ),
        EventInfo::new(names::INVOICE_LINKED, "An electronic invoice was linked to an order."),
        EventInfo::new(names::CART_ABANDONED, "A buyer left a cart without placing an order."),
        EventInfo::new(names::REVIEW_PUBLISHED, "A product review was published."),
        EventInfo::new(names::REVIEW_MODERATED, "A product review was hidden or reopened."),
        EventInfo::new(names::POS_SHIFT_CLOSED, "A cash register shift was closed."),
        EventInfo::new(names::WARRANTY_CLAIM_CHANGED, "A warranty claim changed status."),
        EventInfo::new(names::COD_BLOCK_CHANGED, "The cash-on-delivery block list changed."),
        EventInfo::new(
            names::PREORDER_BALANCE_REQUESTED,
            "The buyer was asked to pay the balance of a pre-order paid in part.",
        ),
        EventInfo::new(
            names::PREORDER_BALANCE_OVERDUE,
            "The balance of a pre-order paid in part was not paid in time.",
        ),
    ]
}
