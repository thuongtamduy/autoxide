//! Kiểm tra sức khoẻ website (bước 67): đọc lần kiểm gần nhất, chạy ngay. Logic ở `health.rs`.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::time::Timestamp;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::health::{Finding, HealthReport};
use crate::service::{ContentService, now};

/// The last check of a website.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct HealthResponse {
    /// The website (organization unit).
    org_id: Uuid,
    #[schema(value_type = String, format = DateTime)]
    checked_at: Timestamp,
    /// Problems found, by kind, at most 200 of each kind; each says how to fix it.
    findings: Vec<Finding>,
}

pub(crate) fn response(row: HealthReport) -> HealthResponse {
    HealthResponse {
        findings: serde_json::from_str(&row.findings).unwrap_or_default(),
        org_id: row.org_id,
        checked_at: row.checked_at,
    }
}

/// Which website.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct HealthQuery {
    /// The website (organization unit).
    org_id: Uuid,
}

/// Get the last health check of a website.
///
/// Requires `content.read` at the unit. `data` is `null` when the website was never checked.
#[utoipa::path(
    get,
    path = "/site-health",
    extensions(("x-permissions" = json!(["content.read"]))),
    operation_id = "get_content_site_health",
    security(("bearer" = [])),
    params(HealthQuery),
    responses(
        (status = 200, description = "The last check, or `null`.", body = Data<Option<HealthResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.read` at the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No unit has this id; code `site_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_site_health(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<HealthQuery>,
) -> Result<Data<Option<HealthResponse>>, Error> {
    Ok(Data { data: service.site_health(&tenant, &principal, query.org_id).await?.map(response) })
}

/// Check the health of a website now.
///
/// Requires `content.write` at the unit; at most once an hour per website. Scans the published
/// posts and pages of the website for a missing SEO description, duplicate titles, images without
/// `alt`, broken links inside the site, embedded records that are no longer public and pages whose
/// countdown is over. The result also feeds the work inbox (`content.site_health`).
#[utoipa::path(
    post,
    path = "/site-health/{org_id}/run",
    extensions(("x-permissions" = json!(["content.write"]))),
    operation_id = "run_content_site_health",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The website (organization unit).")),
    responses(
        (status = 200, description = "The check.", body = Data<HealthResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.write` at the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No unit has this id; code `site_not_found`.", body = Problem),
        (status = 429, description = "Checked less than an hour ago; code `content.site_health_too_soon`.", body = Problem),
    )
)]
pub(crate) async fn run_site_health(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Data<HealthResponse>, Error> {
    Ok(Data { data: response(service.run_site_health(&tenant, &principal, org_id, now()).await?) })
}
