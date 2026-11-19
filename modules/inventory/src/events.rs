//! Event module `inventory` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module; ghi
//! CÙNG transaction với thay đổi (luật kiến trúc 3). Tên event module khác hay nghe là hằng trong
//! `api::events`.

use autoxide_kernel::event::{Event, EventInfo};
use serde::Serialize;
use serde_json::Value;

use crate::api::events as names;

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Default, Serialize)]
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

event!(LevelChanged, names::LEVEL_CHANGED);
event!(LowStock, names::LOW_STOCK);
event!(OutOfStock, names::OUT_OF_STOCK);
event!(MovementRecorded, names::MOVEMENT_RECORDED);
event!(CostChanged, names::COST_CHANGED);
event!(ReservationHeld, names::RESERVATION_HELD);
event!(ReservationConfirmed, names::RESERVATION_CONFIRMED);
event!(ReservationReleased, names::RESERVATION_RELEASED);
event!(ReservationExpired, names::RESERVATION_EXPIRED);
event!(TransferShipped, "inventory.transfer_shipped");
event!(TransferReceived, "inventory.transfer_received");
event!(CountApplied, "inventory.count_applied");

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        EventInfo::new(
            names::LEVEL_CHANGED,
            "The stock of a variant in a warehouse changed (on hand, available).",
        ),
        EventInfo::new(
            names::LOW_STOCK,
            "The available stock of a variant in a warehouse fell to its reorder point or below.",
        ),
        EventInfo::new(
            names::OUT_OF_STOCK,
            "The available stock of a variant in a warehouse fell to zero or below.",
        ),
        EventInfo::new(
            names::MOVEMENT_RECORDED,
            "A line was added to the stock movement ledger (receipt, sale, return, adjustment…).",
        ),
        EventInfo::new(
            names::COST_CHANGED,
            "The weighted average unit cost of a variant in a warehouse changed.",
        ),
        EventInfo::new(names::RESERVATION_HELD, "Stock was reserved for an order."),
        EventInfo::new(
            names::RESERVATION_CONFIRMED,
            "A stock reservation was confirmed: the order was placed.",
        ),
        EventInfo::new(
            names::RESERVATION_RELEASED,
            "A stock reservation was released (order cancelled, payment failed, released by hand).",
        ),
        EventInfo::new(
            names::RESERVATION_EXPIRED,
            "A stock reservation expired before the order was confirmed.",
        ),
        EventInfo::new(
            "inventory.transfer_shipped",
            "A stock transfer left its source warehouse and is in transit.",
        ),
        EventInfo::new(
            "inventory.transfer_received",
            "Goods of a stock transfer arrived at the destination (all or part).",
        ),
        EventInfo::new(
            "inventory.count_applied",
            "A stock count was applied: its differences were written to the ledger.",
        ),
    ]
}

/// Chủ thể (subject) của event về dòng tồn `id`.
pub(crate) fn level_subject(id: uuid::Uuid) -> Option<String> {
    Some(format!("inventory.level:{id}"))
}
