//! Tổng quan tuân thủ (đề bài giao diện, D-20261118-ui-debt-batch-3): số việc đang chờ theo từng khu vực, cho màn đầu của
//! mục Tuân thủ. Mỗi khu vực chỉ có khi người gọi có quyền đọc khu vực đó; khu vực khác là `null`.
//! Mỗi số là một câu `COUNT` theo index của bảng.

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use axum::extract::State;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::entity::{case, incident, kyc_profile, subject_request, website};
use crate::permissions::{CASES_READ, INCIDENTS_MANAGE, KYC_READ, LEGAL_READ, PRIVACY_MANAGE};
use crate::service::{ComplianceService, now};

/// Identity verification waiting for someone.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceOverviewKyc)]
pub(crate) struct KycCounts {
    /// Profiles `pending`: waiting for the provider or for a review.
    pending: u64,
    /// Profiles `needs_info`: waiting for the person to send more.
    needs_info: u64,
}

/// Websites and apps to declare to the Ministry of Industry and Trade.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceOverviewWebsites)]
pub(crate) struct WebsiteCounts {
    /// `not_submitted` or `needs_supplement`: you have something to send.
    to_submit: u64,
    /// `submitted`: waiting for the ministry.
    awaiting_authority: u64,
}

/// Personal data incidents.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceOverviewIncidents)]
pub(crate) struct IncidentCounts {
    /// Not `closed`.
    open: u64,
    /// `open` or `contained` and past the time to report to the authority.
    report_overdue: u64,
}

/// Complaints and infringement reports.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceOverviewCases)]
pub(crate) struct CaseCounts {
    /// Not yet decided, closed or withdrawn (spam held aside not counted).
    open: u64,
    /// Open and past the time to decide.
    overdue: u64,
    /// Held by the anti-spam checks, waiting for a look.
    held: u64,
}

/// Requests of data subjects (access, erasure…).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceOverviewSubjectRequests)]
pub(crate) struct SubjectRequestCounts {
    /// `received` or `in_progress`.
    open: u64,
    /// Open and past the legal deadline.
    overdue: u64,
}

/// Work waiting in each compliance area. An area is `null` when you may not read it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceOverview)]
pub(crate) struct Overview {
    /// Needs `compliance.kyc.read`.
    kyc: Option<KycCounts>,
    /// Needs `compliance.legal.read`; counts the websites of the units in your scope.
    websites: Option<WebsiteCounts>,
    /// Needs `compliance.incidents.manage`.
    incidents: Option<IncidentCounts>,
    /// Needs `compliance.cases.read`.
    cases: Option<CaseCounts>,
    /// Needs `compliance.privacy.manage`.
    subject_requests: Option<SubjectRequestCounts>,
}

impl ComplianceService {
    pub(crate) async fn overview(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Overview, Error> {
        let read = tenant.database().read();
        let at = now();
        let may = |permission: &Permission| principal.has(permission);

        let kyc = if may(&KYC_READ) {
            let count = |status: &str| {
                kyc_profile::Entity::find()
                    .scoped(tenant)
                    .filter(kyc_profile::Column::Status.eq(status))
                    .count(read)
            };
            Some(KycCounts {
                pending: count("pending").await?,
                needs_info: count("needs_info").await?,
            })
        } else {
            None
        };
        let websites = if may(&LEGAL_READ) {
            let scope = principal.scope_condition(&LEGAL_READ, website::Column::OrgId, None)?;
            let count = |statuses: &'static [&'static str]| {
                website::Entity::find()
                    .scoped(tenant)
                    .filter(scope.clone())
                    .filter(website::Column::MoitStatus.is_in(statuses.iter().copied()))
                    .count(read)
            };
            Some(WebsiteCounts {
                to_submit: count(&["not_submitted", "needs_supplement"]).await?,
                awaiting_authority: count(&["submitted"]).await?,
            })
        } else {
            None
        };
        let incidents = if may(&INCIDENTS_MANAGE) {
            let open = incident::Entity::find()
                .scoped(tenant)
                .filter(incident::Column::Status.ne("closed"))
                .count(read)
                .await?;
            let report_overdue = incident::Entity::find()
                .scoped(tenant)
                .filter(incident::Column::Status.is_in(["open", "contained"]))
                .filter(incident::Column::ReportDueAt.lt(at))
                .count(read)
                .await?;
            Some(IncidentCounts { open, report_overdue })
        } else {
            None
        };
        let cases = if may(&CASES_READ) {
            let open = || {
                case::Entity::find()
                    .scoped(tenant)
                    .filter(case::Column::Status.is_not_in(["decided", "closed", "withdrawn"]))
                    .filter(case::Column::SpamReason.is_null())
            };
            let held = case::Entity::find()
                .scoped(tenant)
                .filter(case::Column::Status.eq("received"))
                .filter(case::Column::SpamReason.is_not_null())
                .count(read)
                .await?;
            Some(CaseCounts {
                open: open().count(read).await?,
                overdue: open().filter(case::Column::DecideBy.lt(at)).count(read).await?,
                held,
            })
        } else {
            None
        };
        let subject_requests = if may(&PRIVACY_MANAGE) {
            let open = || {
                subject_request::Entity::find()
                    .scoped(tenant)
                    .filter(subject_request::Column::Status.is_in(["received", "in_progress"]))
            };
            Some(SubjectRequestCounts {
                open: open().count(read).await?,
                overdue: open()
                    .filter(crate::subject_requests::overdue_condition(at))
                    .count(read)
                    .await?,
            })
        } else {
            None
        };
        Ok(Overview { kyc, websites, incidents, cases, subject_requests })
    }
}

/// Work waiting in each compliance area.
///
/// Any signed-in user; each area is filled only for the people who may read it (the permission is
/// named on the area) and is `null` otherwise. For the first screen of the compliance section:
/// identity checks to review, websites to declare, incidents to report, complaints to decide and
/// data subject requests to answer — with how many are past their deadline.
#[utoipa::path(
    get,
    path = "/overview",
    extensions(("x-permissions" = json!([]))),
    operation_id = "compliance_overview",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The counts.", body = Data<Overview>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn overview(
    State(service): State<Arc<ComplianceService>>,
    tenant: TenantContext,
    principal: Principal,
) -> Result<Data<Overview>, Error> {
    Ok(Data { data: service.overview(&tenant, &principal).await? })
}

pub(crate) fn routes(service: Arc<ComplianceService>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(overview)).with_state(service)
}
