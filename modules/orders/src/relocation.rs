//! Chính sách tách / gộp khách của các bảng của `orders` (bước 38, D-20261118-step38-relocation):
//! giỏ và đơn theo nút bán; dòng giỏ, dòng đơn, thuế, lịch sử, thanh toán, hoãn hoàn tất đi theo
//! dòng cha. Lượt đặt (`orders_checkouts`) chỉ là bản ghi chống trùng của `Idempotency-Key`: dữ liệu
//! tạm, không đi theo khách. Test `e2e::relocation::split_and_merge_with_every_module` đỏ khi một
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
        .ephemeral(&["orders_checkouts"])
}
