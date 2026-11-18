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
