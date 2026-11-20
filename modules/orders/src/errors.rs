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

pub(crate) fn cart_empty() -> Error {
    Error::new(ErrorKind::Validation, "orders.cart_empty", "The cart is empty.")
}

pub(crate) fn idempotency_key_required() -> Error {
    Error::new(
        ErrorKind::PreconditionRequired,
        "orders.idempotency_key_required",
        "Placing an order needs the `Idempotency-Key` header (1 to 200 characters): a new key \
         for each attempt, the same key when retrying it.",
    )
}

pub(crate) fn quote_changed() -> Error {
    Error::conflict(
        "orders.quote_changed",
        "Prices, discounts or fees changed: read the cart again and show it to the buyer before \
         ordering.",
    )
}

pub(crate) fn quote_expired() -> Error {
    Error::conflict("orders.quote_expired", "The quote expired: read the cart again.")
}

pub(crate) fn delivery_required() -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.delivery_required",
        "Items must be delivered: set the address and a delivery option of the cart first.",
    )
}

pub(crate) fn payment_method_unavailable(method: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.payment_method_unavailable",
        "The payment method {method} cannot be used for this order.",
    )
    .with_param("method", method)
}

pub(crate) fn promotion_unavailable() -> Error {
    Error::conflict(
        "orders.promotion_unavailable",
        "A promotion of the cart ran out or ended: read the cart again.",
    )
}

pub(crate) fn checkout_in_progress() -> Error {
    Error::conflict(
        "orders.checkout_in_progress",
        "A checkout with this `Idempotency-Key` is still running; retry in a few seconds.",
    )
    .with_retry_after(2)
}

pub(crate) fn checkout_failed(code: &str) -> Error {
    Error::conflict(
        "orders.checkout_failed",
        "The checkout with this `Idempotency-Key` failed ({code}); send a new key to try again.",
    )
    .with_param("code", code)
}

pub(crate) fn checkout_key_reused(key: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.idempotency_key_reused",
        "The `Idempotency-Key` {key} was used for another checkout; send a new key.",
    )
    .with_param("key", key)
}

pub(crate) fn order_not_found() -> Error {
    Error::not_found("orders.order_not_found", "No order with this id.")
}

pub(crate) fn invalid_transition(
    from: crate::api::OrderStatus,
    to: crate::api::OrderStatus,
) -> Error {
    Error::conflict("orders.invalid_transition", "The order is {from}; it cannot move to {to}.")
        .with_param("from", from)
        .with_param("to", to)
}

pub(crate) fn payment_token_invalid() -> Error {
    Error::not_found(
        "orders.payment_token_invalid",
        "No payment with this token, or the payment deadline passed.",
    )
}

pub(crate) fn cancel_not_allowed() -> Error {
    Error::conflict(
        "orders.cancel_not_allowed",
        "The order is past the point where you can cancel it yourself; contact the shop.",
    )
}

pub(crate) fn shipment_not_allowed(status: crate::api::OrderStatus) -> Error {
    Error::conflict(
        "orders.shipment_not_allowed",
        "A shipment cannot be requested while the order is {status}.",
    )
    .with_param("status", status)
}

pub(crate) fn payment_expired() -> Error {
    Error::conflict("orders.payment_expired", "The payment deadline of the order has passed.")
}

pub(crate) fn cod_not_allowed() -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.cod_not_allowed",
        "This order must be paid in advance; cash on delivery is not accepted.",
    )
}

pub(crate) fn refund_exceeds_paid(remaining: autoxide_kernel::money::Money) -> Error {
    Error::conflict(
        "orders.refund_exceeds_paid",
        "The refund is more than what is left to refund ({remaining}); one refund comes from one \
         payment of the order.",
    )
    .with_param("remaining", remaining.to_string())
}

pub(crate) fn return_not_found() -> Error {
    Error::not_found("orders.return_not_found", "No return with this id.")
}

pub(crate) fn return_not_allowed(status: crate::api::OrderStatus) -> Error {
    Error::conflict(
        "orders.return_not_allowed",
        "Items can be returned only from a delivered order; this order is {status}.",
    )
    .with_param("status", status)
}

pub(crate) fn return_window_closed(days: i64) -> Error {
    Error::conflict(
        "orders.return_window_closed",
        "The return window of the order ({days} days after delivery) has ended.",
    )
    .with_param("days", days)
}

pub(crate) fn return_invalid_transition(from: &str, to: &str) -> Error {
    Error::conflict(
        "orders.return_invalid_transition",
        "The return is {from}; it cannot move to {to}.",
    )
    .with_param("from", from.to_owned())
    .with_param("to", to.to_owned())
}

pub(crate) fn pickup_not_allowed(status: crate::api::OrderStatus) -> Error {
    Error::conflict(
        "orders.pickup_not_allowed",
        "This order is not collected in store, or it cannot be handed over while it is {status}.",
    )
    .with_param("status", status)
}

pub(crate) fn pickup_not_ready() -> Error {
    Error::conflict(
        "orders.pickup_not_ready",
        "The order is not ready for collection yet; mark it ready first.",
    )
}

pub(crate) fn pickup_code_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.pickup_code_invalid",
        "The pickup code, or the name and phone number of the person collecting, do not match the \
         order.",
    )
}

pub(crate) fn nothing_to_ship() -> Error {
    Error::conflict(
        "orders.nothing_to_ship",
        "Nothing of the order is ready to ship: pre-order items have not arrived yet, or every \
         item was already requested.",
    )
}

pub(crate) fn lookup_failed() -> Error {
    Error::not_found(
        "orders.lookup_failed",
        "No order with this order number and phone number. Check both and try again.",
    )
}

pub(crate) fn lookup_resend_too_soon(retry_after: u64) -> Error {
    Error::too_many_requests(
        "orders.lookup_resend_too_soon",
        "Wait {retry_after} seconds before asking for another code.",
    )
    .with_param("retry_after", retry_after)
    .with_retry_after(retry_after)
}

pub(crate) fn lookup_too_many(retry_after: u64) -> Error {
    Error::too_many_requests(
        "orders.lookup_too_many",
        "Too many order lookups. Try again in {retry_after} seconds.",
    )
    .with_param("retry_after", retry_after)
    .with_retry_after(retry_after)
}

pub(crate) fn lookup_code_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.lookup_code_invalid",
        "The code is wrong, used or expired; ask for a new code if it has expired.",
    )
}

pub(crate) fn lookup_token_invalid() -> Error {
    Error::unauthenticated(
        "orders.lookup_token_invalid",
        "The order access token is missing, wrong or expired. Look the order up again with its \
         number and phone number, then send the token in the `Order-Token` header.",
    )
}

pub(crate) fn address_locked() -> Error {
    Error::conflict(
        "orders.address_locked",
        "The goods have left the warehouse (or a shipment was requested); the delivery address \
         can no longer change.",
    )
}

pub(crate) fn address_change_needs_payment() -> Error {
    Error::conflict(
        "orders.address_change_needs_payment",
        "The new address changes the shipping fee (or the shipping method is not available there); \
         contact the shop to change it.",
    )
}

pub(crate) fn support_unavailable() -> Error {
    Error::not_found("orders.support_unavailable", "This shop has no customer support turned on.")
}

pub(crate) fn guest_key_reused(key: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.idempotency_key_reused",
        "The `Idempotency-Key` {key} was used for another request; send a new key.",
    )
    .with_param("key", key)
}

pub(crate) fn sms_unavailable() -> Error {
    autoxide_kernel::sms::sms_disabled()
}

pub(crate) fn invoices_unavailable() -> Error {
    Error::not_found(
        "orders.invoices_unavailable",
        "This server issues no electronic invoices (the `invoices` module is not installed).",
    )
}

pub(crate) fn invoice_already_issued() -> Error {
    Error::conflict(
        "orders.invoice_already_issued",
        "The order already has an invoice; ask the shop for an adjustment or a replacement.",
    )
}

pub(crate) fn document_not_found() -> Error {
    Error::not_found("orders.document_not_found", "No such document of this order.")
}

pub(crate) fn shift_not_found() -> Error {
    Error::not_found("orders.shift_not_found", "No cash register shift with this id.")
}

pub(crate) fn shift_already_open(register: &str) -> Error {
    Error::conflict(
        "orders.shift_already_open",
        "Register {register} already has an open shift; close it first.",
    )
    .with_param("register", register)
}

pub(crate) fn shift_not_open() -> Error {
    Error::conflict("orders.shift_not_open", "The shift is not open (not opened yet, or closed).")
}

pub(crate) fn shift_not_pending() -> Error {
    Error::conflict("orders.shift_not_pending", "The shift is not waiting for approval.")
}

pub(crate) fn shift_approval_self() -> Error {
    Error::forbidden(
        "orders.shift_approval_self",
        "The person who closed the shift cannot approve its cash difference; ask someone else.",
    )
}

pub(crate) fn payments_mismatch(
    paid: autoxide_kernel::money::Money,
    total: autoxide_kernel::money::Money,
) -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.payments_mismatch",
        "The payments add up to {paid}, the order total is {total}.",
    )
    .with_param("paid", paid.to_string())
    .with_param("total", total.to_string())
}

pub(crate) fn review_not_found() -> Error {
    Error::not_found("orders.review_not_found", "No product review with this id.")
}

pub(crate) fn review_not_allowed() -> Error {
    Error::forbidden(
        "orders.review_not_allowed",
        "Only buyers who received this item can review it (the order must be delivered).",
    )
}

pub(crate) fn review_exists() -> Error {
    Error::conflict(
        "orders.review_exists",
        "You already reviewed this item of this order; edit that review instead.",
    )
}

pub(crate) fn review_edit_closed() -> Error {
    Error::conflict("orders.review_edit_closed", "The time to edit this review has passed.")
}

pub(crate) fn review_vote_own() -> Error {
    Error::conflict("orders.review_vote_own", "You cannot vote for your own review.")
}

pub(crate) fn review_vote_not_allowed() -> Error {
    Error::forbidden(
        "orders.review_vote_not_allowed",
        "Sellers and moderators cannot vote for reviews of their own products.",
    )
}

pub(crate) fn review_votes_too_many(retry_after: u64) -> Error {
    Error::too_many_requests(
        "orders.review_votes_too_many",
        "Too many votes. Try again in {retry_after} seconds.",
    )
    .with_param("retry_after", retry_after)
    .with_retry_after(retry_after)
}

pub(crate) fn moderation_reason_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "orders.moderation_reason_invalid",
        "Hiding a review needs a reason of the list `orders.review_hidden`.",
    )
}

pub(crate) fn account_not_verified() -> Error {
    Error::forbidden(
        "orders.account_not_verified",
        "Verify your account (phone number) before writing reviews.",
    )
}

pub(crate) fn ai_unavailable() -> Error {
    Error::unavailable(
        "orders.ai_unavailable",
        "AI summaries are not available: the `ai` module is not installed or turned off.",
    )
}

pub(crate) fn summary_not_found() -> Error {
    Error::not_found("orders.summary_not_found", "No review summary with this id.")
}

pub(crate) fn summary_not_draft() -> Error {
    Error::conflict(
        "orders.summary_not_draft",
        "Only a generated draft can be approved or rejected.",
    )
}

pub(crate) fn cod_rule_not_found() -> Error {
    Error::not_found("orders.cod_rule_not_found", "No cash-on-delivery rule with this id.")
}

pub(crate) fn cod_block_not_found() -> Error {
    Error::not_found("orders.cod_block_not_found", "No cash-on-delivery block with this id.")
}
