//! Cảnh báo vận hành cho nhân sự nền tảng (đề bài giao diện, màn Sức khoẻ): chạy lại các bước
//! kiểm của `autoxide doctor` mà module khai (`Module::checks`) và trả những bước chưa đạt, mỗi
//! bước kèm câu "làm gì" (`hint`). Không có bảng cảnh báo riêng: sự thật là kết quả kiểm lúc này.
//!
//! Bước kiểm có thể gọi mạng (máy chủ mail, nơi lưu file) nên kết quả giữ trong bộ nhớ 60 giây;
//! `refresh=true` kiểm lại ngay. Các bước chạy song song, mỗi bước có hạn giờ của doctor.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::access::Principal;
use crate::doctor::Status;
use crate::error::Error;
use crate::ops::module_catalog::ModuleCatalog;
use crate::response::{Data, Problem};

/// Giữ kết quả kiểm bao lâu.
const CACHE_FOR: Duration = Duration::from_secs(60);

/// Something about the server that needs attention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct Alert {
    /// The check that found it, `<module>.<check>` (as in `autoxide doctor`).
    #[schema(example = "mail.smtp")]
    pub name: String,
    /// `failed` (what depends on it does not work) or `warning` (works, but fix it before
    /// running for real).
    #[schema(example = "warning")]
    pub severity: String,
    /// What was found, in English.
    pub message: String,
    /// What to do about it (a setting, a command), in English; `null` when the check gives none.
    pub hint: Option<String>,
}

/// The current alerts.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct AlertsResponse {
    /// When the checks ran (up to a minute ago; `refresh=true` runs them now).
    pub checked_at: DateTime<Utc>,
    /// How many checks ran.
    pub checks: usize,
    /// Failed ones first, then warnings; empty when everything is fine.
    pub alerts: Vec<Alert>,
}

/// Whether to run the checks again.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct AlertsQuery {
    /// `true`: run every check now instead of using the result of the last minute.
    refresh: Option<bool>,
}

/// Bộ chạy kiểm dùng chung cho mọi request, kèm kết quả lần gần nhất.
pub(crate) struct Alerts {
    catalog: ModuleCatalog,
    last: Mutex<Option<(Instant, AlertsResponse)>>,
}

impl Alerts {
    pub(crate) fn new(catalog: ModuleCatalog) -> Self {
        Alerts { catalog, last: Mutex::new(None) }
    }

    async fn current(&self, refresh: bool) -> AlertsResponse {
        if !refresh {
            let last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, response)) = last.as_ref()
                && at.elapsed() < CACHE_FOR
            {
                return response.clone();
            }
        }
        let checks = self.catalog.checks();
        // Không giữ lock qua `await`: hai request cùng lúc có thể cùng kiểm, chỉ tốn thêm một lượt.
        let findings = futures_util::future::join_all(
            checks
                .iter()
                .map(|(module, check)| async move { (*module, check.name, check.run().await) }),
        )
        .await;
        let mut alerts: Vec<Alert> = findings
            .into_iter()
            .filter_map(|(module, name, finding)| {
                let severity = match finding.status {
                    Status::Ok => return None,
                    Status::Warning => "warning",
                    Status::Failed => "failed",
                };
                Some(Alert {
                    name: format!("{module}.{name}"),
                    severity: severity.to_owned(),
                    message: finding.message,
                    hint: finding.hint,
                })
            })
            .collect();
        // `failed` trước `warning`, rồi theo tên.
        alerts.sort_by(|a, b| {
            (a.severity != "failed", &a.name).cmp(&(b.severity != "failed", &b.name))
        });
        let response = AlertsResponse { checked_at: Utc::now(), checks: checks.len(), alerts };
        *self.last.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), response.clone()));
        response
    }
}

pub(crate) fn routes(alerts: Arc<Alerts>) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(list_alerts)).with_state(alerts)
}

/// List what needs attention on the server.
///
/// Requires `kernel.tenants.read`, as staff of the platform tenant. Runs the checks of
/// `autoxide doctor` that modules declare (mail server, file storage, signing keys, tax rates
/// about to expire…) and lists those that are not fine, each with what to do. Results are kept
/// for a minute; `refresh=true` runs the checks now.
#[utoipa::path(
    get,
    path = "/ops/alerts",
    extensions(("x-permissions" = json!(["kernel.tenants.read"]))),
    operation_id = "list_ops_alerts",
    security(("bearer" = [])),
    params(AlertsQuery),
    responses(
        (status = 200, description = "The alerts; empty when everything is fine.", body = Data<AlertsResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `kernel.tenants.read` (`permission_denied`), or not staff of the platform tenant (`tenant_platform_only`).", body = Problem),
    )
)]
async fn list_alerts(
    State(alerts): State<Arc<Alerts>>,
    principal: Principal,
    Query(query): Query<AlertsQuery>,
) -> Result<Data<AlertsResponse>, Error> {
    principal.require(&crate::tenancy::TENANTS_READ)?;
    crate::tenancy::require_platform(&principal)?;
    Ok(Data { data: alerts.current(query.refresh.unwrap_or(false)).await })
}
