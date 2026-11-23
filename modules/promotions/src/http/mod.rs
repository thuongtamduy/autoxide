//! REST của module `promotions`, dưới `/api/v1/promotions`. Mọi route đăng ký qua `routes!` để có
//! tài liệu OpenAPI (luật kiến trúc 6).

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;

use crate::service::PromotionsService;

/// Endpoint của người mua (ví voucher, nhập mã, lưu voucher, nhắc flash sale). Tài khoản người mua
/// không có permission nào (D-309) nên chỉ cần là người mua; tài khoản nhân sự (vd thử trên giao diện
/// quản trị) cần `promotions.use` như trước.
pub(crate) fn require_use(
    principal: &autoxide_kernel::access::Principal,
) -> Result<(), autoxide_kernel::error::Error> {
    if principal.realm() == autoxide_kernel::access::Realm::Customer {
        return Ok(());
    }
    principal.require(&crate::permissions::USE)
}

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
