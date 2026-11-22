//! Endpoint REST của module `shipping`, dưới `/api/v1/shipping`. Handler chỉ: lấy dữ liệu vào →
//! gọi service → trả envelope.

pub(crate) mod addresses;
mod calendars;
mod carriers;
pub(crate) mod cod;
pub(crate) mod quote;
mod settings;
pub(crate) mod shipments;

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
        .routes(routes!(carriers::list_accounts, carriers::create_account))
        .routes(routes!(carriers::update_account))
        .routes(routes!(carriers::test_account))
        .routes(routes!(carriers::receive_webhook))
        .routes(routes!(carriers::receive_webhook_for_tenant))
        .routes(routes!(shipments::list_shipments, shipments::create_shipment))
        .routes(routes!(shipments::get_shipment))
        .routes(routes!(shipments::book_shipment))
        .routes(routes!(shipments::cancel_shipment))
        .routes(routes!(shipments::split_shipment))
        .routes(routes!(shipments::set_parcels))
        .routes(routes!(shipments::set_status))
        .routes(routes!(shipments::get_label))
        .routes(routes!(cod::import_statement, cod::list_statements))
        .routes(routes!(cod::get_statement))
        .routes(routes!(cod::list_discrepancies))
        .routes(routes!(cod::resolve_discrepancy))
        .routes(routes!(cod::refresh_tracking))
        .routes(routes!(calendars::list_calendars, calendars::create_calendar))
        .routes(routes!(calendars::get_calendar, calendars::update_calendar))
        .routes(routes!(calendars::set_day, calendars::delete_day))
        .routes(routes!(calendars::estimate))
        .with_state(service)
}
