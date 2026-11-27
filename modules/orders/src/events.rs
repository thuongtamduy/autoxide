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
event!(PickupReady, names::PICKUP_READY);
event!(PreorderDelayed, names::PREORDER_DELAYED);
event!(PreorderCancelled, names::PREORDER_CANCELLED);
event!(PickupReminder, names::PICKUP_REMINDER);
event!(PickupExpired, names::PICKUP_EXPIRED);
event!(ReturnRequested, names::RETURN_REQUESTED);
event!(ReturnPickupRequested, names::RETURN_PICKUP_REQUESTED);
event!(ReturnReceived, names::RETURN_RECEIVED);
event!(ReturnStatusChanged, names::RETURN_STATUS_CHANGED);
event!(DeliveryAddressChanged, names::DELIVERY_ADDRESS_CHANGED);
event!(OrdersClaimed, names::ORDERS_CLAIMED);
event!(InvoiceLinked, names::INVOICE_LINKED);
event!(PosShiftClosed, names::POS_SHIFT_CLOSED);
event!(ReviewPublished, names::REVIEW_PUBLISHED);
event!(ReviewModerated, names::REVIEW_MODERATED);
event!(CodBlockChanged, names::COD_BLOCK_CHANGED);
event!(WarrantyClaimChanged, names::WARRANTY_CLAIM_CHANGED);
event!(PreorderBalanceRequested, names::PREORDER_BALANCE_REQUESTED);
event!(PreorderBalanceOverdue, names::PREORDER_BALANCE_OVERDUE);

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
        EventInfo::new(
            names::PAYMENT_RECORDED,
            "A payment of an order was recorded. `late: true`: it arrived after the order was cancelled \
             and is refunded. `superseded: true`: it is an attempt replaced by a newer one; it does not \
             count towards the order and is refunded through the payment partner.",
        ),
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
        EventInfo::new(
            names::PICKUP_READY,
            "An order to collect in store is ready; `after.pickup_code` is the code to show.",
        ),
        EventInfo::new(
            names::PICKUP_REMINDER,
            "An order waiting in store will stop being held within 24 hours.",
        ),
        EventInfo::new(
            names::PICKUP_EXPIRED,
            "An order waiting in store was not collected in time.",
        ),
        EventInfo::new(
            names::PREORDER_DELAYED,
            "A pre-order item will arrive later than promised; the buyer may cancel for free.",
        ),
        EventInfo::new(
            names::PREORDER_CANCELLED,
            "Pre-order items not yet arrived were cancelled from an order and refunded.",
        ),
        EventInfo::new(
            names::DELIVERY_ADDRESS_CHANGED,
            "The buyer changed the delivery address of an order before it left the warehouse.",
        ),
        EventInfo::new(
            names::ORDERS_CLAIMED,
            "Guest orders were attached to the account of the buyer who proved the phone number.",
        ),
        EventInfo::new(names::INVOICE_LINKED, "An electronic invoice was linked to an order."),
        EventInfo::new(names::CART_ABANDONED, "A buyer left a cart without placing an order."),
        EventInfo::new(names::REVIEW_PUBLISHED, "A product review was published."),
        EventInfo::new(names::REVIEW_MODERATED, "A product review was hidden or reopened."),
        EventInfo::new(names::POS_SHIFT_CLOSED, "A cash register shift was closed."),
        EventInfo::new(names::COD_BLOCK_CHANGED, "The cash-on-delivery block list changed."),
        EventInfo::new(names::WARRANTY_CLAIM_CHANGED, "A warranty claim changed status."),
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

/// Số liệu cho báo cáo gộp (bước 38): doanh số đơn đã xác nhận và tiền đã hoàn, theo kênh.
pub(crate) const SALES_TOTAL: autoxide_kernel::report::FactKind =
    autoxide_kernel::report::FactKind::new(
        "orders.sales_total",
        "Total of the orders confirmed (paid, cash on delivery accepted, point of sale or channel), \
         in the currency of each order; dimension `channel` is `web`, `pos` or a channel code.",
        autoxide_kernel::report::Measure::Amount,
        &["channel"],
    );

pub(crate) const SALES_COUNT: autoxide_kernel::report::FactKind =
    autoxide_kernel::report::FactKind::new(
        "orders.sales_count",
        "Number of orders confirmed; dimension `channel`.",
        autoxide_kernel::report::Measure::Quantity,
        &["channel"],
    );

pub(crate) const REFUNDED_TOTAL: autoxide_kernel::report::FactKind =
    autoxide_kernel::report::FactKind::new(
        "orders.refunded_total",
        "Money refunded to buyers, in the currency of each order; dimension `channel`.",
        autoxide_kernel::report::Measure::Amount,
        &["channel"],
    );

pub(crate) fn facts() -> Vec<autoxide_kernel::report::FactSource> {
    vec![
        autoxide_kernel::report::FactSource::new(
            names::ORDER_CONFIRMED,
            vec![SALES_TOTAL, SALES_COUNT],
            confirmed_facts,
        ),
        autoxide_kernel::report::FactSource::new(
            names::REFUND_COMPLETED,
            vec![REFUNDED_TOTAL],
            refunded_facts,
        ),
    ]
}

fn channel_of(after: &Value) -> String {
    after.get("channel").and_then(Value::as_str).unwrap_or("web").to_owned()
}

/// `orders.order_confirmed` → tổng tiền và một đơn, theo kênh.
fn confirmed_facts(
    event: &autoxide_kernel::event::Delivered,
) -> Result<Vec<autoxide_kernel::report::Fact>, autoxide_kernel::error::Error> {
    use autoxide_kernel::report::{Fact, money_field, org_field};
    let after = &event.payload["after"];
    let org = org_field(after);
    let channel = channel_of(after);
    let mut facts = Vec::new();
    if let Some(total) = money_field(after, "total")? {
        facts.push(
            Fact::new(SALES_TOTAL.kind, event.occurred_at)
                .org(org)
                .amount(total)
                .dimension("channel", channel.clone()),
        );
    }
    facts.push(
        Fact::new(SALES_COUNT.kind, event.occurred_at)
            .org(org)
            .quantity(1)
            .dimension("channel", channel),
    );
    Ok(facts)
}

/// `orders.refund_completed` → số tiền hoàn, theo kênh.
fn refunded_facts(
    event: &autoxide_kernel::event::Delivered,
) -> Result<Vec<autoxide_kernel::report::Fact>, autoxide_kernel::error::Error> {
    use autoxide_kernel::report::{Fact, money_field, org_field};
    let after = &event.payload["after"];
    let Some(amount) = money_field(after, "amount")? else { return Ok(Vec::new()) };
    Ok(vec![
        Fact::new(REFUNDED_TOTAL.kind, event.occurred_at)
            .org(org_field(after))
            .amount(amount)
            .dimension("channel", channel_of(after)),
    ])
}
