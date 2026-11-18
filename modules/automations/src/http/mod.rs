//! Endpoint REST của module `automations`, dưới `/api/v1/automations`. Handler chỉ: lấy dữ liệu vào
//! → kiểm permission → gọi service → trả envelope. Mọi route đăng ký qua `routes!` để có tài liệu
//! OpenAPI (luật kiến trúc 6).

pub(crate) mod catalog;
pub(crate) mod rules;
pub(crate) mod runs;

use std::sync::Arc;

use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::service::Automations;

pub(crate) fn routes(service: Arc<Automations>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(rules::list_rules, rules::create_rule))
        .routes(routes!(rules::get_rule, rules::update_rule, rules::delete_rule))
        .routes(routes!(rules::enable_rule))
        .routes(routes!(rules::disable_rule))
        .routes(routes!(runs::run_rule))
        .routes(routes!(runs::list_runs))
        .routes(routes!(runs::get_run))
        .routes(routes!(runs::retry_run))
        .routes(routes!(runs::approve_run))
        .routes(routes!(runs::reject_run))
        .routes(routes!(catalog::test_rule))
        .routes(routes!(catalog::get_catalog))
        .routes(routes!(catalog::list_templates))
        .with_state(service)
}
