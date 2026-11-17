//! Số liệu tóm tắt (bước 51 P4, D-207): module khai số liệu của mình (tiền, số đếm, tỉ lệ) kèm câu
//! **định nghĩa** — tính từ lúc nào, gồm gì — nằm cạnh code tính số, để hộp ⓘ của giao diện luôn
//! đúng. `GET /api/v1/kernel/summary?group=` trả số liệu của một nhóm điều hướng (hay mọi nhóm, cho
//! trang Tổng quan) mà người gọi có permission, mỗi số có hạn giờ như hộp việc.
//!
//! Định nghĩa là bắt buộc ([`MetricInfo::new`] nhận nó); nhãn và định nghĩa viết tiếng Anh, dịch qua
//! `text:` của module, và `xtask` kiểm có bản dịch như mô tả permission.

use std::sync::{Arc, PoisonError, RwLock};

use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{OrgTarget, Permission, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::Error;
use crate::i18n::{Language, Translations};
use crate::response::{Data, Problem};
use crate::tenancy::{OrgId, TenantContext, root_of};

/// Phần khai của một số liệu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetricInfo {
    /// `<module>.<số liệu>`, vd `comments.pending`.
    pub id: &'static str,
    /// Nhóm điều hướng (P1) của số liệu, vd `marketing`.
    pub group: &'static str,
    pub label: &'static str,
    /// Câu định nghĩa: tính thế nào, đúng như code của `value`.
    pub definition: &'static str,
}

impl MetricInfo {
    pub const fn new(
        id: &'static str,
        label: &'static str,
        definition: &'static str,
        group: &'static str,
    ) -> Self {
        MetricInfo { id, group, label, definition }
    }
}

/// Giá trị của một số liệu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricValue {
    /// Tiền, đơn vị nhỏ nhất của `currency` (như `Money`).
    Money {
        amount_minor: i64,
        currency: String,
    },
    Count(u64),
    Ratio {
        numerator: u64,
        denominator: u64,
    },
}

/// Một số liệu module khai bằng [`StartContext::summary_metric`](crate::module::StartContext).
/// Số nặng đọc từ bảng dẫn xuất, không quét bảng giao dịch mỗi lần mở trang.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait SummaryMetric: Send + Sync {
    fn info(&self) -> MetricInfo;
    fn permission(&self) -> Permission;
    async fn value(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
    ) -> Result<MetricValue, Error>;
}

type Entry = (&'static str, Arc<dyn SummaryMetric>);

/// Sổ số liệu của mọi module. Clone rẻ.
#[derive(Clone, Default)]
pub struct SummaryMetrics {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl SummaryMetrics {
    pub(crate) fn register(
        &self,
        module: &'static str,
        metric: Arc<dyn SummaryMetric>,
    ) -> Result<(), Error> {
        let info = metric.info();
        if !info.id.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "summary metric `{}` of module `{module}` must be named `{module}.<name>`",
                info.id
            )));
        }
        if info.definition.trim().is_empty() {
            return Err(Error::internal(format!("summary metric `{}` has no definition", info.id)));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.info().id == info.id) {
            return Err(Error::internal(format!(
                "summary metric `{}` is registered twice",
                info.id
            )));
        }
        entries.push((module, metric));
        Ok(())
    }

    fn all(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// A figure for a dashboard.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct MetricView {
    #[schema(example = "comments.pending")]
    pub id: String,
    /// Navigation group of the figure.
    pub group: String,
    /// Label in the language of `Accept-Language`.
    pub label: String,
    /// How it is computed (period, time zone, what is included), in the language of
    /// `Accept-Language`: show it behind an ⓘ.
    pub definition: String,
    pub value: MetricValueView,
}

/// The value of a figure: `money` (`amount` in the smallest unit of `currency`), `count`, or
/// `ratio` (`numerator` / `denominator`).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct MetricValueView {
    #[schema(example = "count")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numerator: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub denominator: Option<u64>,
}

impl From<MetricValue> for MetricValueView {
    fn from(value: MetricValue) -> Self {
        let empty = MetricValueView {
            kind: String::new(),
            amount: None,
            currency: None,
            count: None,
            numerator: None,
            denominator: None,
        };
        match value {
            MetricValue::Money { amount_minor, currency } => MetricValueView {
                kind: "money".to_owned(),
                amount: Some(amount_minor),
                currency: Some(currency),
                ..empty
            },
            MetricValue::Count(count) => {
                MetricValueView { kind: "count".to_owned(), count: Some(count), ..empty }
            }
            MetricValue::Ratio { numerator, denominator } => MetricValueView {
                kind: "ratio".to_owned(),
                numerator: Some(numerator),
                denominator: Some(denominator),
                ..empty
            },
        }
    }
}

/// The figures.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct SummaryView {
    pub metrics: Vec<MetricView>,
    /// Figures that did not answer in time or failed.
    pub incomplete: Vec<String>,
}

/// Which figures.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SummaryQuery {
    /// Only the figures of this navigation group (e.g. `marketing`); omitted: every group.
    group: Option<String>,
    /// The organization unit chosen; omitted: the whole tenant.
    org_id: Option<Uuid>,
}

#[derive(Clone)]
pub(crate) struct SummaryState {
    pub metrics: SummaryMetrics,
    pub entitlements: EntitlementsHandle,
    pub translations: Translations,
}

impl SummaryState {
    async fn summary(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        query: &SummaryQuery,
        language: Language,
    ) -> Result<SummaryView, Error> {
        let target =
            OrgTarget::new(query.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid), None);
        let read = tenant.database().read();
        let mut allowed = Vec::new();
        for (module, metric) in self.metrics.all() {
            if query.group.as_deref().is_some_and(|group| group != metric.info().group) {
                continue;
            }
            if self.entitlements.module_enabled(tenant.id(), module).await?
                && principal.require_at(read, &metric.permission(), target).await.is_ok()
            {
                allowed.push(metric);
            }
        }
        let answers = futures_util::future::join_all(allowed.iter().map(|metric| async move {
            let value = tokio::time::timeout(
                crate::ops::work::SOURCE_TIMEOUT,
                metric.value(tenant, principal, target),
            )
            .await;
            (metric.info(), value)
        }))
        .await;
        let mut metrics = Vec::new();
        let mut incomplete = Vec::new();
        for (info, answer) in answers {
            match answer {
                Ok(Ok(value)) => metrics.push(MetricView {
                    id: info.id.to_owned(),
                    group: info.group.to_owned(),
                    label: self.translations.describe(info.label, language),
                    definition: self.translations.describe(info.definition, language),
                    value: value.into(),
                }),
                Ok(Err(error)) => {
                    tracing::warn!(metric = info.id, error = %error, "summary metric failed");
                    incomplete.push(info.id.to_owned());
                }
                Err(_) => {
                    tracing::warn!(metric = info.id, "summary metric timed out");
                    incomplete.push(info.id.to_owned());
                }
            }
        }
        Ok(SummaryView { metrics, incomplete })
    }
}

pub(crate) fn routes(state: SummaryState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(get_summary)).with_state(Arc::new(state))
}

/// Get the summary figures.
///
/// Any signed-in user. The figures modules declare (counts, money, ratios) for a navigation
/// group, or for every group when `group` is omitted (the overview page), among those the caller
/// has permission for at `org_id`. Each figure carries its `definition` — how it is computed —
/// in the caller's language. A slow or failing figure is skipped and named in `incomplete`.
#[utoipa::path(
    get,
    path = "/summary",
    extensions(("x-permissions" = json!([]))),
    operation_id = "get_summary",
    security(("bearer" = [])),
    params(SummaryQuery),
    responses(
        (status = 200, description = "The figures.", body = Data<SummaryView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn get_summary(
    State(state): State<Arc<SummaryState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Query(query): Query<SummaryQuery>,
) -> Result<Data<SummaryView>, Error> {
    Ok(Data { data: state.summary(&tenant, &principal, &query, language).await? })
}
