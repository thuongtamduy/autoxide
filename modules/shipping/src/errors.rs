//! Lỗi của module `shipping`. `code` là hợp đồng với client và là i18n key: bản dịch tiếng Việt ở
//! `locales/vi.json`. Lỗi bộ nối hãng cũng dùng (`carrier_error`…) nằm ở `api::errors`.

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use uuid::Uuid;

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
