//! Endpoint REST của module `support`, dưới `/api/v1/support`. Handler chỉ: lấy dữ liệu vào → gọi
//! service → trả envelope. Ba nhóm: nhân viên (`/tickets`, theo permission ở nút), người mua có tài
//! khoản (`/me/…`, chỉ phiếu của mình), khách vãng lai (`/public/…`, bằng token phiếu).

mod me;
mod public;
mod settings;
mod staff;
pub(crate) mod views;

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::SupportService;

pub(crate) fn routes(service: Arc<SupportService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(staff::list_tickets, staff::create_ticket))
        .routes(routes!(staff::get_ticket, staff::update_ticket))
        .routes(routes!(staff::add_message))
        .routes(routes!(staff::hide_message))
        .routes(routes!(me::list_my_tickets))
        .routes(routes!(me::get_my_ticket))
        .routes(routes!(me::add_my_message))
        .routes(routes!(me::claim_tickets))
        .routes(routes!(public::open_ticket))
        .routes(routes!(public::get_public_ticket))
        .routes(routes!(public::add_public_message))
        .routes(routes!(public::verify_ticket))
        .routes(routes!(settings::list_teams, settings::create_team))
        .routes(routes!(settings::get_team, settings::delete_team))
        .routes(routes!(settings::set_member, settings::remove_member))
        .routes(routes!(settings::list_rules, settings::create_rule))
        .routes(routes!(settings::delete_rule))
        .with_state(service)
}
