//! Event module `shipping` phát. Cùng hình dạng `{"before": …, "after": …}` với mọi module; ghi
//! CÙNG transaction với thay đổi (luật kiến trúc 3). Tên event là hằng trong `api::events` để
//! module khác (`orders`) nghe đúng tên. Payload của event vận đơn là `ShipmentEvent`: chỉ ID và
//! trạng thái, không mang tên, số điện thoại hay địa chỉ người nhận.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Event, EventInfo, Outbox};
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::ConnectionTrait;
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::api::events as names;

/// Dữ liệu chung của mọi event của module.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct Change {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Value>,
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

event!(ShipmentCreated, names::SHIPMENT_CREATED);
event!(ShipmentBooked, names::SHIPMENT_BOOKED);
event!(ShipmentStatusChanged, names::SHIPMENT_STATUS_CHANGED);
event!(ShipmentDelivered, names::SHIPMENT_DELIVERED);
event!(ShipmentDeliveryFailed, names::SHIPMENT_DELIVERY_FAILED);
event!(ShipmentReturned, names::SHIPMENT_RETURNED);
event!(ShipmentCancelled, names::SHIPMENT_CANCELLED);
event!(BookingFailed, names::BOOKING_FAILED);
event!(CodCollected, names::COD_COLLECTED);
event!(CarrierAccountChanged, names::CARRIER_ACCOUNT_CHANGED);

pub(crate) fn all() -> Vec<EventInfo> {
    vec![
        EventInfo::new(
            names::SHIPMENT_CREATED,
            "A shipment was created (pending), from an order or by staff.",
        ),
        EventInfo::new(
            names::SHIPMENT_BOOKED,
            "A shipment was booked with its carrier and has a tracking number.",
        ),
        EventInfo::new(
            names::SHIPMENT_STATUS_CHANGED,
            "The status of a shipment changed (any change).",
        ),
        EventInfo::new(names::SHIPMENT_DELIVERED, "A shipment was delivered to the buyer."),
        EventInfo::new(names::SHIPMENT_DELIVERY_FAILED, "A delivery attempt failed."),
        EventInfo::new(names::SHIPMENT_RETURNED, "A shipment came back to the shop."),
        EventInfo::new(names::SHIPMENT_CANCELLED, "A shipment was cancelled before pickup."),
        EventInfo::new(
            names::BOOKING_FAILED,
            "The carrier refused a booking; the shipment is pending again.",
        ),
        EventInfo::new(
            names::COD_COLLECTED,
            "The carrier collected the cash on delivery of a shipment.",
        ),
        EventInfo::new(
            names::CARRIER_ACCOUNT_CHANGED,
            "A carrier account was connected or changed (never with its credentials).",
        ),
    ]
}

/// Ghi event theo tên (tên thuộc `api::events`), cho chỗ chọn tên lúc chạy.
pub(crate) trait RecordNamed {
    async fn record_named<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        name: &'static str,
        subject: Option<String>,
        change: &Change,
    ) -> Result<(), Error>;
}

impl RecordNamed for Outbox {
    async fn record_named<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        actor: Actor,
        name: &'static str,
        subject: Option<String>,
        change: &Change,
    ) -> Result<(), Error> {
        let change = change.clone();
        macro_rules! go {
            ($kind:ident) => {
                self.record(connection, tenant, actor, subject, &$kind(change)).await
            };
        }
        match name {
            names::SHIPMENT_CREATED => go!(ShipmentCreated),
            names::SHIPMENT_BOOKED => go!(ShipmentBooked),
            names::SHIPMENT_STATUS_CHANGED => go!(ShipmentStatusChanged),
            names::SHIPMENT_DELIVERED => go!(ShipmentDelivered),
            names::SHIPMENT_DELIVERY_FAILED => go!(ShipmentDeliveryFailed),
            names::SHIPMENT_RETURNED => go!(ShipmentReturned),
            names::SHIPMENT_CANCELLED => go!(ShipmentCancelled),
            names::BOOKING_FAILED => go!(BookingFailed),
            names::COD_COLLECTED => go!(CodCollected),
            names::CARRIER_ACCOUNT_CHANGED => go!(CarrierAccountChanged),
            other => Err(Error::internal(format!("event `{other}` is not declared by `shipping`"))),
        }
    }
}

/// Chủ thể của event về vận đơn `id`.
pub(crate) fn shipment_subject(id: Uuid) -> Option<String> {
    Some(format!("shipping.shipment:{id}"))
}

/// Chủ thể của event về tài khoản hãng `id`.
pub(crate) fn account_subject(id: Uuid) -> Option<String> {
    Some(format!("shipping.carrier_account:{id}"))
}
