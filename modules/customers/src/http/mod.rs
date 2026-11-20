//! Endpoint REST của module `customers`, dưới `/api/v1/customers`. Handler chỉ: lấy dữ liệu vào → gọi
//! service → trả envelope. Ba nhóm: nhân viên (hồ sơ, theo permission ở nút), nhóm khách
//! (`/groups`), người mua (`/me`, chỉ hồ sơ của mình).

mod engagement;
mod groups;
mod loyalty;
mod me;
mod staff;
mod stored_value;
pub(crate) mod views;
mod wishlist_alerts;

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::CustomersService;

pub(crate) fn routes(service: Arc<CustomersService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(staff::list_customers, staff::register_customer))
        .routes(routes!(staff::get_customer, staff::update_customer))
        .routes(routes!(staff::reveal_contact))
        .routes(routes!(staff::list_customer_orders))
        .routes(routes!(groups::list_groups, groups::create_group))
        .routes(routes!(groups::get_group, groups::update_group, groups::delete_group))
        .routes(routes!(groups::preview_group))
        .routes(routes!(groups::add_member, groups::remove_member))
        .routes(routes!(me::get_me, me::update_me))
        .routes(routes!(me::list_my_orders))
        .routes(routes!(loyalty::list_rules, loyalty::create_rule))
        .routes(routes!(loyalty::list_tiers, loyalty::create_tier))
        .routes(routes!(loyalty::list_rewards, loyalty::create_reward))
        .routes(routes!(loyalty::set_reward))
        .routes(routes!(loyalty::get_points))
        .routes(routes!(loyalty::list_points_entries))
        .routes(routes!(loyalty::adjust_points))
        .routes(routes!(loyalty::get_my_points))
        .routes(routes!(loyalty::list_my_rewards))
        .routes(routes!(loyalty::redeem_voucher))
        .routes(routes!(engagement::list_wishlist, engagement::add_wishlist))
        .routes(routes!(engagement::merge_wishlist))
        .routes(routes!(wishlist_alerts::get_wishlist_alerts, wishlist_alerts::set_wishlist_alerts))
        .routes(routes!(engagement::remove_wishlist))
        .routes(routes!(engagement::list_views, engagement::record_view))
        .routes(routes!(engagement::my_referral))
        .routes(routes!(engagement::apply_referral))
        .routes(routes!(engagement::list_reminder_rules, engagement::create_reminder_rule))
        .routes(routes!(engagement::list_referral_rules, engagement::create_referral_rule))
        .routes(routes!(stored_value::issue_gift_card))
        .routes(routes!(stored_value::get_gift_card))
        .routes(routes!(stored_value::lookup_gift_card))
        .routes(routes!(stored_value::void_gift_card))
        .routes(routes!(stored_value::adjust_store_credit))
        .routes(routes!(stored_value::my_gift_cards))
        .routes(routes!(stored_value::attach_gift_card))
        .routes(routes!(stored_value::my_store_credit))
        .with_state(service)
}
