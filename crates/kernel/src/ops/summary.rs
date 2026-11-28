//! Summary figures (step 51 P4, D-207): a module declares its figures (money, counts, ratios) together with a
//! **definition** sentence (since when it is computed, what it includes) kept next to the code that computes
//! the figure, so the interface's info box is always correct. `GET /api/v1/kernel/summary?group=` returns the
//! figures of one navigation group (or every group, for the Overview page) that the caller has permission for,
//! each figure with a timeout like the work inbox.
//!
//! The definition is required ([`MetricInfo::new`] takes it); the label and definition are written in
//! English, translated through the module's `text:`, and `xtask` checks that translations exist, like
//! permission descriptions.

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

/// Declaration part of a figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetricInfo {
    /// `<module>.<figure>`, e.g. `comments.pending`.
    pub id: &'static str,
    /// Navigation group (P1) of the figure, e.g. `marketing`.
    pub group: &'static str,
    pub label: &'static str,
    /// Definition sentence: how it is computed, exactly as the code of `value` does.
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

/// Value of a figure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricValue {
    /// Money, in the smallest unit of `currency` (like `Money`).
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

/// A figure that a module declares with [`StartContext::summary_metric`](crate::module::StartContext).
/// Heavy figures are read from derived tables, not by scanning the transaction table on every page open.
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

/// Registry of every module's figures. Cheap to clone.
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

    /// Figure `id` (`<module>.<figure>`), if a module declares it; lets a module read another module's figures
    /// through a shared contract (e.g. anomaly alerts of `automations`, step 58).
    pub fn find(&self, id: &str) -> Option<Arc<dyn SummaryMetric>> {
        self.all().into_iter().map(|(_, metric)| metric).find(|metric| metric.info().id == id)
    }

    /// Declaration part of every figure, in registration order.
    pub fn infos(&self) -> Vec<MetricInfo> {
        self.all().iter().map(|(_, metric)| metric.info()).collect()
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
    /// Time limit of one source (`AUTOXIDE_SOURCE_TIMEOUT_MS`).
    pub timeout: std::time::Duration,
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
        let timeout = self.timeout;
        let answers = futures_util::future::join_all(allowed.iter().map(|metric| async move {
            let value =
                tokio::time::timeout(timeout, metric.value(tenant, principal, target)).await;
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
