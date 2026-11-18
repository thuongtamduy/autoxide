//! Endpoint REST của module `shipping`, dưới `/api/v1/shipping`. Handler chỉ: lấy dữ liệu vào →
//! gọi service → trả envelope.

mod addresses;

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
        .with_state(service)
}
