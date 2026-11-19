//! REST của module `promotions`, dưới `/api/v1/promotions`. Mọi route đăng ký qua `routes!` để có
//! tài liệu OpenAPI (luật kiến trúc 6).

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::service::PromotionsService;

pub(crate) mod admin;
pub(crate) mod buyer;
pub(crate) mod dto;
pub(crate) mod insights;
pub(crate) mod notices;
pub(crate) mod participations;
pub(crate) mod public;
pub(crate) mod reminders;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(admin::routes(service.clone()))
        .merge(notices::routes(service.clone()))
        .merge(participations::routes(service.clone()))
        .merge(insights::routes(service.clone()))
        .merge(public::routes(service.clone()))
        .merge(reminders::routes(service.clone()))
        .merge(buyer::routes(service))
}
