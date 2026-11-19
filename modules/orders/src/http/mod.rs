//! Endpoint REST của module `orders`, dưới `/api/v1/orders`. Handler chỉ: lấy dữ liệu vào → gọi
//! service → trả envelope. Người mua đăng nhập (bearer) hay vãng lai (`Cart-Token`, token xem
//! đơn); nhân viên theo permission ở nút.

mod carts;
pub(crate) mod dto;
mod lifecycle;
mod orders;
pub(crate) mod returns;

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
        .routes(routes!(orders::checkout))
        .routes(routes!(orders::public_payment))
        .routes(routes!(orders::my_orders))
        .routes(routes!(orders::my_order))
        .routes(routes!(orders::list_orders))
        .routes(routes!(orders::get_order))
        .routes(routes!(lifecycle::transition))
        .routes(routes!(lifecycle::cancel_mine))
        .routes(routes!(lifecycle::request_shipment))
        .routes(routes!(lifecycle::payment_link))
        .routes(routes!(returns::request_return))
        .routes(routes!(returns::my_return))
        .routes(routes!(returns::list_returns))
        .routes(routes!(returns::get_return))
        .routes(routes!(returns::transition_return))
        .routes(routes!(returns::refund_order, returns::order_refunds))
        .with_state(service)
}
