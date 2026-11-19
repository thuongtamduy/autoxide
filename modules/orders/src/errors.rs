//! Lỗi của module `orders`. `code` là hợp đồng với client và là i18n key (bản dịch ở
//! `locales/vi.json`); câu tiếng Anh ở đây là bản mặc định.

use autoxide_kernel::error::{Error, ErrorKind};

pub(crate) fn cart_not_found() -> Error {
    Error::not_found("orders.cart_not_found", "No cart with this id.")
}

pub(crate) fn cart_token_invalid() -> Error {
    Error::unauthenticated(
        "orders.cart_token_invalid",
        "The cart token is missing, wrong or expired. Send the `Cart-Token` header received when \
         the cart was created, or sign in.",
    )
}

pub(crate) fn cart_too_many_lines(max: usize) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.cart_too_many_lines",
        "A cart holds at most {max} products.",
    )
    .with_param("max", max)
}

pub(crate) fn quantity_invalid(max: i64) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.quantity_invalid",
        "The quantity must be from 1 to {max}.",
    )
    .with_param("max", max)
}

pub(crate) fn variant_unavailable(sku: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.variant_unavailable",
        "The product {sku} is not on sale now.",
    )
    .with_param("sku", sku)
}

pub(crate) fn out_of_stock(sku: &str, available: i64) -> Error {
    Error::conflict(
        "orders.out_of_stock",
        "Not enough of the product {sku} in stock ({available} left).",
    )
    .with_param("sku", sku)
    .with_param("available", available)
}

pub(crate) fn preorder_closed() -> Error {
    Error::conflict("orders.preorder_closed", "Pre-orders for this product have closed.")
}

pub(crate) fn vouchers_need_account() -> Error {
    Error::forbidden(
        "orders.vouchers_need_account",
        "Vouchers live in the wallet of an account: sign in to use them.",
    )
}

pub(crate) fn cart_closed() -> Error {
    Error::conflict(
        "orders.cart_closed",
        "This cart was already merged or turned into an order; use the current cart.",
    )
}
