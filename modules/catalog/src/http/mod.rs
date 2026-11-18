//! REST của module `catalog`, dưới `/api/v1/catalog`. Mọi route đăng ký qua `routes!` để có tài
//! liệu OpenAPI (luật kiến trúc 6).

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::service::CatalogService;

pub(crate) mod attributes;
pub(crate) mod changes;
pub(crate) mod dto;
pub(crate) mod pricing;
pub(crate) mod products;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(products::routes(service.clone()))
        .merge(pricing::routes(service.clone()))
        .merge(attributes::routes(service.clone()))
        .merge(changes::routes(service))
}
