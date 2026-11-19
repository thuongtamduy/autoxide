//! Chính sách tách / gộp khách của các bảng của `orders` (bước 38, D-20261118-step38-relocation):
//! giỏ và đơn theo nút bán; dòng giỏ, dòng đơn, thuế, lịch sử, thanh toán, lượt đặt đi theo cha. Mỗi mục của bước 45 thêm bảng của mình vào đây; test
//! `e2e::relocation::split_and_merge_with_every_module` đỏ khi một bảng có `tenant_id` bị quên.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new().by_org(&["orders_carts"]).follow(
        "orders_cart_lines",
        "cart_id",
        "orders_carts",
    )
}
