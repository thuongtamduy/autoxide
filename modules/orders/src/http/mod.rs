//! Endpoint REST của module `orders`, dưới `/api/v1/orders`. Handler chỉ: lấy dữ liệu vào → gọi
//! service → trả envelope. Người mua đăng nhập (bearer) hay vãng lai (`Cart-Token`, token xem
//! đơn); nhân viên theo permission ở nút.

mod carts;
mod dto;

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::OrdersService;

pub(crate) fn routes(service: Arc<OrdersService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(carts::list_carts, carts::create_cart))
        .routes(routes!(carts::get_cart))
        .routes(routes!(carts::set_line))
        .routes(routes!(carts::set_codes))
        .routes(routes!(carts::set_vouchers))
        .routes(routes!(carts::set_delivery))
        .routes(routes!(carts::set_payment_method))
        .routes(routes!(carts::set_attribution))
        .routes(routes!(carts::cart_promotions))
        .routes(routes!(carts::merge_cart))
        .with_state(service)
}
