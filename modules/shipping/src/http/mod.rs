//! Endpoint REST của module `shipping`, dưới `/api/v1/shipping`. Handler chỉ: lấy dữ liệu vào →
//! gọi service → trả envelope.

pub(crate) mod addresses;
mod quote;
mod settings;

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::ShippingService;

pub(crate) fn routes(service: Arc<ShippingService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(addresses::list_my_addresses, addresses::create_my_address))
        .routes(routes!(
            addresses::get_my_address,
            addresses::update_my_address,
            addresses::delete_my_address
        ))
        .routes(routes!(addresses::set_my_default_address))
        .routes(routes!(addresses::list_buyer_addresses))
        .routes(routes!(settings::list_zones, settings::create_zone))
        .routes(routes!(settings::get_zone, settings::update_zone, settings::delete_zone))
        .routes(routes!(settings::set_zone_regions))
        .routes(routes!(settings::list_methods, settings::create_method))
        .routes(routes!(settings::get_method, settings::update_method, settings::delete_method))
        .routes(routes!(settings::set_method_rates))
        .routes(routes!(settings::set_method_pickup_points))
        .routes(routes!(quote::quote))
        .with_state(service)
}
