//! Xuất dữ liệu thông báo khuyến mại theo từng nơi nhận (D-225), JSON hay CSV.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::http::admin::csv_response;
use crate::notices::NoticeExport;
use crate::permissions::COMPLIANCE;
use crate::service::PromotionsService;

pub(crate) fn routes(service: Arc<PromotionsService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(export_notice)).with_state(service)
}

/// Format of an export.
#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = PromotionsExportFormat)]
pub(crate) enum ExportFormat {
    #[default]
    Json,
    Csv,
}

/// Format of the notice export.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct ExportQuery {
    /// `json` (default) or `csv` (one line per authority, safe to open in a spreadsheet).
    #[param(inline)]
    pub format: Option<ExportFormat>,
}

/// Export the data of a promotion notice, one entry per authority.
///
/// Requires `promotions.compliance` and `promotions.read`. For each authority that must be told
/// of the promotion under the rules in force on its start day (e.g. `province:79`, the
/// Department of Industry and Trade of that province): whether a notice is required, the last day
/// to submit, the legal basis and what was recorded with
/// `PATCH /api/v1/promotions/{id}/notices/{authority_code}`. People submit the notice; this only
/// prepares the data. `rules_found` is `false` when no law data is in force for the promotion.
#[utoipa::path(
    get,
    path = "/promotions/{id}/notice-export",
    extensions(("x-permissions" = json!(["promotions.compliance", "promotions.read"]))),
    operation_id = "export_promotion_notice",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the promotion."), ExportQuery),
    responses(
        (status = 200, description = "The notice data: JSON (`format=json`) or a CSV file (`format=csv`).", content(
            (Data<NoticeExport> = "application/json"),
            (String = "text/csv"),
        )),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `promotions.compliance` or `promotions.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such promotion in your scope; code `promotions.not_found`.", body = Problem),
    )
)]
async fn export_notice(
    State(service): State<Arc<PromotionsService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    Query(query): Query<ExportQuery>,
) -> Result<Response, Error> {
    principal.require(&COMPLIANCE)?;
    let export = service.notice_export(&tenant, &principal, id).await?;
    Ok(match query.format.unwrap_or_default() {
        ExportFormat::Json => Data { data: export }.into_response(),
        ExportFormat::Csv => csv_response(export.to_csv(), &format!("promotion-{id}-notice.csv")),
    })
}
