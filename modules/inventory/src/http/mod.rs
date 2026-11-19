//! REST của module `inventory`, dưới `/api/v1/inventory`. Mọi route đăng ký qua `routes!` để có
//! tài liệu OpenAPI (luật kiến trúc 6). Handler chỉ: lấy dữ liệu vào → kiểm permission → gọi
//! service → trả envelope.

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::service::InventoryService;

pub(crate) mod documents;
pub(crate) mod dto;
pub(crate) mod levels;
pub(crate) mod public;
pub(crate) mod purchasing;
pub(crate) mod reservations;

pub(crate) fn routes(service: Arc<InventoryService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .merge(levels::routes(service.clone()))
        .merge(reservations::routes(service.clone()))
        .merge(documents::routes(service.clone()))
        .merge(purchasing::routes(service.clone()))
        .merge(public::routes(service))
}
