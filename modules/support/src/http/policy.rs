//! Chính sách hỗ trợ theo nút và báo cáo (bước 53 mục 3).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::permissions::{SETTINGS_MANAGE, TICKETS_READ, TICKETS_REPLY};
use crate::policy::{Effective, Policy};
use crate::report::Summary;
use crate::service::SupportService;

/// The policy in force at a unit, and where it comes from.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[schema(as = SupportPolicyInForce)]
pub(crate) struct PolicyResponse {
    /// The unit asked about.
    org_id: Uuid,
    /// The unit whose own policy applies; empty: the defaults.
    defined_at: Option<Uuid>,
    #[serde(flatten)]
    policy: Policy,
}

impl PolicyResponse {
    fn new(org_id: Uuid, effective: Effective) -> Self {
        PolicyResponse { org_id, defined_at: effective.from, policy: effective.policy }
    }
}

/// A policy to set on a unit (every field; units below without their own policy inherit it).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = SetSupportPolicyRequest)]
#[serde(deny_unknown_fields)]
pub(crate) struct PolicyRequest {
    #[serde(flatten)]
    policy: Policy,
}

/// Get the support policy in force at a unit.
///
/// Requires `support.tickets.reply` or `support.settings.manage` (staff need the categories and
/// response-time targets). Units without their own policy show the inherited one (`defined_at`).
#[utoipa::path(
    get,
    path = "/policies/{org_id}",
    extensions(("x-permissions" = json!(["support.tickets.reply"]))),
    operation_id = "get_support_policy",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The unit.")),
    responses(
        (status = 200, description = "The policy in force.", body = Data<PolicyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.reply`; code `permission_denied`.", body = Problem),
    )
)]
pub(crate) async fn get_policy(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Data<PolicyResponse>, Error> {
    if !principal.has(&SETTINGS_MANAGE) {
        principal.require(&TICKETS_REPLY)?;
    }
    let effective = service.policy(&tenant, org_id).await?;
    Ok(Data { data: PolicyResponse::new(org_id, effective) })
}

/// Set the support policy of a unit.
///
/// Requires `support.settings.manage` at the unit. Every error of the policy is reported at once.
/// New tickets use it at once; open tickets keep their deadlines until their priority changes.
#[utoipa::path(
    put,
    path = "/policies/{org_id}",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "set_support_policy",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The unit.")),
    request_body = PolicyRequest,
    responses(
        (status = 200, description = "The policy now in force.", body = Data<PolicyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
        (status = 422, description = "A value is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn set_policy(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
    ValidJson(input): ValidJson<PolicyRequest>,
) -> Result<Data<PolicyResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let effective = service.set_policy(&tenant, &principal, org_id, input.policy).await?;
    Ok(Data { data: PolicyResponse::new(org_id, effective) })
}

/// Remove the own support policy of a unit.
///
/// Requires `support.settings.manage` at the unit. The unit then inherits the policy of the units
/// above (or the defaults).
#[utoipa::path(
    delete,
    path = "/policies/{org_id}",
    extensions(("x-permissions" = json!(["support.settings.manage"]))),
    operation_id = "reset_support_policy",
    security(("bearer" = [])),
    params(("org_id" = Uuid, Path, description = "The unit.")),
    responses(
        (status = 200, description = "The policy now in force.", body = Data<PolicyResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.settings.manage` (`permission_denied`), the unit is outside your scope (`org_out_of_scope`) or not active (`org_frozen`).", body = Problem),
    )
)]
pub(crate) async fn reset_policy(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(org_id): Path<Uuid>,
) -> Result<Data<PolicyResponse>, Error> {
    principal.require(&SETTINGS_MANAGE)?;
    let effective = service.reset_policy(&tenant, &principal, org_id).await?;
    Ok(Data { data: PolicyResponse::new(org_id, effective) })
}

/// The period and units of a report.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ReportQuery {
    /// Tickets created from this moment (included).
    from: DateTime<Utc>,
    /// … to this moment (excluded).
    to: DateTime<Utc>,
    /// Only this unit and the units below it.
    org_id: Option<Uuid>,
}

/// Support figures for a period.
///
/// Requires `support.tickets.read`; counts the tickets in your scope created in the period: how
/// many, how many solved and late, median minutes to the first answer and to solved, and the
/// average customer rating.
#[utoipa::path(
    get,
    path = "/reports/summary",
    extensions(("x-permissions" = json!(["support.tickets.read"]))),
    operation_id = "support_report_summary",
    security(("bearer" = [])),
    params(ReportQuery),
    responses(
        (status = 200, description = "The figures.", body = Data<Summary>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `support.tickets.read`; code `permission_denied`.", body = Problem),
        (status = 422, description = "`to` is not after `from`; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn report_summary(
    State(service): State<Arc<SupportService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<ReportQuery>,
) -> Result<Data<Summary>, Error> {
    principal.require(&TICKETS_READ)?;
    let summary = service.summary(&tenant, &principal, query.org_id, query.from, query.to).await?;
    Ok(Data { data: summary })
}
