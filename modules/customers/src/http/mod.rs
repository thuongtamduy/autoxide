//! Endpoint REST của module `customers`, dưới `/api/v1/customers`. Handler chỉ: lấy dữ liệu vào → gọi
//! service → trả envelope. Ba nhóm: nhân viên (hồ sơ, theo permission ở nút), nhóm khách
//! (`/groups`), người mua (`/me`, chỉ hồ sơ của mình).

mod groups;
mod loyalty;
mod me;
mod staff;
pub(crate) mod views;

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
        .with_state(service)
}
