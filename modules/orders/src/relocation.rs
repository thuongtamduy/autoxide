//! Chính sách tách / gộp khách của các bảng của `orders` (bước 38, D-20261118-step38-relocation):
//! giỏ và đơn theo nút bán; dòng giỏ, dòng đơn, thuế, lịch sử, thanh toán, hoãn hoàn tất, hoàn tiền, đổi trả đi theo
//! dòng cha. Lượt đặt (`orders_checkouts`) chỉ là bản ghi chống trùng của `Idempotency-Key`: dữ liệu
//! tạm, không đi theo khách; mã một lần, token xem đơn, chống trùng thao tác vãng lai và bảng đếm
//! giới hạn cũng vậy (sống vài chục phút). Test `e2e::relocation::split_and_merge_with_every_module` đỏ khi một
//! bảng có `tenant_id` bị quên.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&["orders_carts", "orders_orders"])
        .follow("orders_cart_lines", "cart_id", "orders_carts")
        .follow("orders_lines", "order_id", "orders_orders")
        .follow("orders_line_taxes", "line_id", "orders_lines")
        .follow("orders_status_history", "order_id", "orders_orders")
        .follow("orders_payments", "order_id", "orders_orders")
        .follow("orders_completion_holds", "order_id", "orders_orders")
        .follow("orders_refunds", "order_id", "orders_orders")
        .follow("orders_returns", "order_id", "orders_orders")
        .follow("orders_pickups", "order_id", "orders_orders")
        .follow("orders_fulfillments", "order_id", "orders_orders")
        .ephemeral(&[
            "orders_checkouts",
            "orders_lookup_codes",
            "orders_lookup_tokens",
            "orders_guest_requests",
            "orders_throttle",
        ])
}
