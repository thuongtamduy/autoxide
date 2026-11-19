//! Lỗi của module `shipping`. `code` là hợp đồng với client và là i18n key: bản dịch tiếng Việt ở
//! `locales/vi.json`. Lỗi bộ nối hãng cũng dùng (`carrier_error`…) nằm ở `api::errors`.

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use uuid::Uuid;

use crate::api::ShipmentStatus;

pub(crate) fn address_not_found(id: Uuid) -> Error {
    Error::not_found(
        "shipping.address_not_found",
        "There is no address `{id}` in your address book.",
    )
    .with_param("id", id)
}

pub(crate) fn address_limit(max: u64) -> Error {
    Error::conflict(
        "shipping.address_limit",
        "A buyer keeps at most {max} addresses. Delete one before adding another.",
    )
    .with_param("max", max)
}

/// `shipping.invalid_address` (422) kèm lỗi theo trường.
pub(crate) fn invalid_address(fields: Vec<FieldError>) -> Error {
    let problem = fields.first().map(|field| field.field.clone()).unwrap_or_default();
    let mut error = Error::new(
        ErrorKind::Validation,
        "shipping.invalid_address",
        "The address is not valid: {problem}.",
    )
    .with_param("problem", problem);
    for field in fields {
        error = error.with_field(field);
    }
    error
}

pub(crate) fn zone_not_found(id: Uuid) -> Error {
    Error::not_found("shipping.zone_not_found", "There is no shipping zone `{id}` in your scope.")
        .with_param("id", id)
}

pub(crate) fn method_not_found(id: Uuid) -> Error {
    Error::not_found(
        "shipping.method_not_found",
        "There is no shipping method `{id}` in your scope.",
    )
    .with_param("id", id)
}

pub(crate) fn method_code_taken(code: &str) -> Error {
    Error::conflict(
        "shipping.method_code_taken",
        "This unit already has a shipping method with the code `{code}`. Choose another code.",
    )
    .with_param("code", code)
}

pub(crate) fn rate_overlap(zone: Uuid) -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.rate_overlap",
        "The weight tiers of zone `{zone}` overlap: each weight must fall in one tier.",
    )
    .with_param("zone", zone)
}

pub(crate) fn not_pickup_method() -> Error {
    Error::conflict("shipping.not_pickup_method", "Pickup points belong to `pickup` methods only.")
}

pub(crate) fn option_changed() -> Error {
    Error::conflict(
        "shipping.option_changed",
        "The shipping fee or destination changed; choose the shipping option again.",
    )
}

pub(crate) fn carrier_unknown(code: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.carrier_unknown",
        "No connector for the carrier `{carrier}` is installed.",
    )
    .with_param("carrier", code)
}

pub(crate) fn carrier_account_not_found(id: Uuid) -> Error {
    Error::not_found(
        "shipping.carrier_account_not_found",
        "There is no carrier account `{id}` in your scope.",
    )
    .with_param("id", id)
}

pub(crate) fn shipment_not_found(id: Uuid) -> Error {
    Error::not_found("shipping.shipment_not_found", "There is no shipment `{id}` in your scope.")
        .with_param("id", id)
}

/// Hai lệnh cùng tạo một vận đơn (unique `order_id` + `request_key`): bên gọi đọc lại.
pub(crate) fn request_duplicate() -> Error {
    Error::conflict(
        "shipping.request_duplicate",
        "This shipment request was received at the same time by another call.",
    )
}

pub(crate) fn request_conflict(key: &str) -> Error {
    Error::conflict(
        "shipping.request_conflict",
        "The request key `{key}` was already used for this order with other content.",
    )
    .with_param("key", key)
}

pub(crate) fn invalid_transition(from: ShipmentStatus, to: ShipmentStatus) -> Error {
    Error::conflict("shipping.invalid_transition", "A shipment cannot go from `{from}` to `{to}`.")
        .with_param("from", from)
        .with_param("to", to)
}

pub(crate) fn already_booked() -> Error {
    Error::conflict("shipping.already_booked", "The shipment is already booked with the carrier.")
}

pub(crate) fn not_booked() -> Error {
    Error::conflict("shipping.not_booked", "The shipment is not booked with a carrier yet.")
}

pub(crate) fn no_carrier() -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.no_carrier",
        "The shipping method of this shipment has no carrier account.",
    )
}

pub(crate) fn booking_rejected(code: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.booking_rejected",
        "The carrier refused the booking (`{reason}`); fix the shipment and book again.",
    )
    .with_param("reason", code)
}

pub(crate) fn webhook_too_large(max: usize) -> Error {
    Error::new(
        ErrorKind::PayloadTooLarge,
        "shipping.webhook_too_large",
        "The webhook body is larger than {max} bytes.",
    )
    .with_param("max", max)
}

pub(crate) fn webhook_unknown() -> Error {
    Error::not_found(
        "shipping.webhook_unknown",
        "No active carrier account has this webhook address.",
    )
}

pub(crate) fn webhook_body() -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.webhook_body",
        "The webhook body is not UTF-8 text.",
    )
}

pub(crate) fn statement_duplicate(reference: &str) -> Error {
    Error::conflict(
        "shipping.statement_duplicate",
        "The statement `{reference}` was already imported with other content.",
    )
    .with_param("reference", reference)
}

pub(crate) fn statement_not_available() -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.statement_not_available",
        "This carrier gives no statement through its API; import the CSV file instead.",
    )
}

pub(crate) fn statement_not_found(id: Uuid) -> Error {
    Error::not_found("shipping.statement_not_found", "There is no COD statement `{id}`.")
        .with_param("id", id)
}

pub(crate) fn discrepancy_not_found(id: Uuid) -> Error {
    Error::not_found("shipping.discrepancy_not_found", "There is no COD discrepancy `{id}`.")
        .with_param("id", id)
}

pub(crate) fn discrepancy_resolved() -> Error {
    Error::conflict("shipping.discrepancy_resolved", "This COD discrepancy is already resolved.")
}

pub(crate) fn calendar_not_found(id: Uuid) -> Error {
    Error::not_found(
        "shipping.calendar_not_found",
        "There is no delivery calendar `{id}` in your scope.",
    )
    .with_param("id", id)
}

pub(crate) fn estimate_unavailable() -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.estimate_unavailable",
        "Shipping estimates need the catalog module, which is not installed.",
    )
}

pub(crate) fn estimate_quantity() -> Error {
    Error::new(
        ErrorKind::Validation,
        "shipping.estimate_quantity",
        "The quantity must be 1 to 1000.",
    )
}

pub(crate) fn variant_not_for_sale(id: Uuid) -> Error {
    Error::not_found("shipping.variant_not_for_sale", "There is no variant `{id}` on sale.")
        .with_param("id", id)
}
