//! Endpoint REST của module `orders`, dưới `/api/v1/orders`. Handler chỉ: lấy dữ liệu vào → gọi
//! service → trả envelope. Người mua đăng nhập (bearer) hay vãng lai (`Cart-Token`, token xem
//! đơn); nhân viên theo permission ở nút.

mod carts;
pub(crate) mod dto;
mod guests;
mod invoices;
mod lifecycle;
mod orders;
mod pickups;
mod pos;
pub(crate) mod returns;
mod reviews;
mod risk;
pub(crate) mod warranty;

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
        .routes(routes!(lifecycle::pay_balance))
        .routes(routes!(pickups::ready))
        .routes(routes!(pickups::handover))
        .routes(routes!(returns::request_return))
        .routes(routes!(returns::preview_return))
        .routes(routes!(returns::my_return))
        .routes(routes!(returns::list_returns))
        .routes(routes!(returns::get_return))
        .routes(routes!(returns::transition_return))
        .routes(routes!(returns::refund_order, returns::order_refunds))
        .routes(routes!(guests::lookup))
        .routes(routes!(guests::confirm_lookup))
        .routes(routes!(guests::get_guest_order))
        .routes(routes!(guests::cancel_guest_order))
        .routes(routes!(guests::send_address_code))
        .routes(routes!(guests::change_address))
        .routes(routes!(guests::guest_support))
        .routes(routes!(guests::my_order_support))
        .routes(routes!(guests::claim))
        .routes(routes!(invoices::request_invoice))
        .routes(routes!(invoices::request_my_invoice))
        .routes(routes!(invoices::my_documents))
        .routes(routes!(invoices::guest_documents))
        .routes(routes!(invoices::print_my_document))
        .routes(routes!(invoices::print_guest_document))
        .routes(routes!(pos::open_shift))
        .routes(routes!(pos::close_shift))
        .routes(routes!(pos::approve_shift))
        .routes(routes!(pos::quote))
        .routes(routes!(pos::sell))
        .routes(routes!(pos::catalog))
        .routes(routes!(reviews::create_review, reviews::edit_review))
        .routes(routes!(reviews::public_reviews))
        .routes(routes!(reviews::review_stats))
        .routes(routes!(reviews::vote_helpful, reviews::unvote_helpful))
        .routes(routes!(reviews::staff_reviews))
        .routes(routes!(reviews::count_reviews))
        .routes(routes!(reviews::moderate))
        .routes(routes!(reviews::reply))
        .routes(routes!(reviews::request_summary))
        .routes(routes!(reviews::approve_summary))
        .routes(routes!(reviews::reject_summary))
        .routes(routes!(risk::buyer_risk))
        .routes(routes!(risk::list_rules, risk::create_rule))
        .routes(routes!(risk::end_rule))
        .routes(routes!(risk::list_blocks, risk::create_block))
        .routes(routes!(risk::lift_block))
        .routes(routes!(risk::sales_report))
        .routes(routes!(warranty::record_serials))
        .routes(routes!(warranty::find_warranties))
        .routes(routes!(warranty::my_warranties))
        .routes(routes!(warranty::create_claim))
        .routes(routes!(warranty::transition_claim))
        .with_state(service)
}
