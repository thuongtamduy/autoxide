//! Hộp việc và số đếm menu (bước 51 P2, D-212): module khai **nguồn việc** tính từ dữ liệu của
//! mình (bình luận chờ duyệt, bài nộp mới, bài chờ duyệt…) — không có bảng việc, không có "đã xử
//! lý": xử lý xong bản ghi thì việc tự biến mất.
//!
//! - `GET /api/v1/kernel/work` gom việc từ các nguồn người gọi có permission, sắp theo mức gấp rồi
//!   hạn. Mỗi nguồn có hạn giờ ([`SOURCE_TIMEOUT`]); nguồn chậm hay lỗi bị bỏ qua và ghi vào
//!   `incomplete`, không làm hỏng cả hộp.
//! - `GET /api/v1/kernel/work/counts` trả `{nguồn: số}` cho số đếm trên menu (`NavItem::counter`),
//!   giữ trong bộ nhớ [`COUNT_CACHE`] với khoá gồm khách, người gọi, nút.
//! - Khác hộp thư của `notifications` (tin đã gửi cho một người).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
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

/// Hạn giờ của một nguồn việc trong một lần gom.
pub const SOURCE_TIMEOUT: Duration = Duration::from_millis(300);
/// Giữ số đếm trong bộ nhớ bao lâu.
const COUNT_CACHE: Duration = Duration::from_secs(10);
/// Tối đa số việc mỗi nguồn trả trong một lần gom.
const PER_SOURCE: u16 = 50;

/// How urgent a piece of work is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// It blocks something else (e.g. selling): do it now.
    Blocking,
    /// Someone has to handle it.
    Attention,
    /// Good to know.
    Info,
}

/// Một việc tính từ dữ liệu của module. Câu viết tiếng Anh (có `{tên}`); kernel dịch qua bản dịch
/// `text:` của module như mô tả permission (D-264).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WorkItem {
    /// ID bản ghi trong nguồn: khoá ổn định của việc là `<nguồn>:<record_id>`.
    pub record_id: String,
    pub severity: Severity,
    pub title: &'static str,
    /// Giá trị điền vào `{tên}` của `title` (dữ liệu, không dịch).
    pub params: Vec<(&'static str, String)>,
    pub due_at: Option<DateTime<Utc>>,
    /// Màn của sổ điều hướng mở việc này (`NavItem::id`).
    pub screen: &'static str,
}

impl WorkItem {
    pub fn new(
        record_id: impl Into<String>,
        severity: Severity,
        title: &'static str,
        screen: &'static str,
    ) -> Self {
        WorkItem {
            record_id: record_id.into(),
            severity,
            title,
            params: Vec::new(),
            due_at: None,
            screen,
        }
    }

    pub fn param(mut self, name: &'static str, value: impl ToString) -> Self {
        self.params.push((name, value.to_string()));
        self
    }

    pub fn due_at(mut self, due_at: Option<DateTime<Utc>>) -> Self {
        self.due_at = due_at;
        self
    }
}

/// Một nguồn việc module khai bằng [`StartContext::work_source`](crate::module::StartContext).
///
/// Câu đếm phải chạy trên index. Nguồn tự áp phạm vi của người gọi (`principal`) ở nút `target`.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait WorkSource: Send + Sync {
    /// `<module>.<việc>`, vd `comments.pending`; cũng là `counter` của mục menu.
    fn id(&self) -> &'static str;
    /// Người gọi cần permission này (ở nút đang chọn) mới thấy nguồn.
    fn permission(&self) -> Permission;
    /// Mọi người đã đăng nhập đều thấy nguồn (bỏ qua [`permission`](Self::permission)); nguồn tự
    /// chỉ trả việc của người gọi — vd yêu cầu duyệt mà người gọi quyết được.
    fn for_any_signed_in(&self) -> bool {
        false
    }
    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
    ) -> Result<u64, Error>;
    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error>;
}

type Entry = (&'static str, Arc<dyn WorkSource>);

/// Sổ nguồn việc của mọi module. Clone rẻ: mọi bản clone thấy cùng một sổ.
#[derive(Clone, Default)]
pub struct WorkSources {
    // Ghi chỉ lúc khởi động; đọc mỗi lần gom. Không giữ khoá qua `await`.
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl WorkSources {
    /// Đăng ký `source` của `module`: id phải là `<module>.<…>` và chưa ai dùng.
    pub(crate) fn register(
        &self,
        module: &'static str,
        source: Arc<dyn WorkSource>,
    ) -> Result<(), Error> {
        let id = source.id();
        if !id.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "work source `{id}` of module `{module}` must be named `{module}.<name>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.id() == id) {
            return Err(Error::internal(format!("work source `{id}` is registered twice")));
        }
        entries.push((module, source));
        Ok(())
    }

    fn all(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Nguồn có id `id`, kèm module của nó.
    pub(crate) fn find(&self, id: &str) -> Option<Entry> {
        self.all().into_iter().find(|(_, source)| source.id() == id)
    }
}

/// One piece of work.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct WorkItemView {
    /// Stable key, `<source>:<record id>`.
    #[schema(example = "comments.pending:0190a1b2-…")]
    pub key: String,
    #[schema(example = "comments.pending")]
    pub source: String,
    pub severity: Severity,
    /// What to do, in the language of `Accept-Language`.
    pub title: String,
    pub due_at: Option<DateTime<Utc>>,
    /// Where to do it: the screen of `GET /api/v1/kernel/navigation` and the record.
    pub target: WorkTarget,
    /// Who it is assigned to (`POST /api/v1/me/work/{key}/assign`).
    pub assignee_id: Option<Uuid>,
    /// Hidden from your inbox until then (only with `include_snoozed=true`).
    pub snoozed_until: Option<DateTime<Utc>>,
}

/// Where a piece of work is done.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct WorkTarget {
    /// Screen id of the navigation, e.g. `marketing.comments`.
    pub screen: String,
    pub record_id: String,
}

/// The inbox.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct WorkView {
    /// Most urgent first (`blocking`, `attention`, `info`), then soonest due.
    pub items: Vec<WorkItemView>,
    /// Sources that did not answer in time or failed: the list may miss their work.
    pub incomplete: Vec<String>,
}

/// Counts for the menu.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct WorkCounts {
    /// Pending work by source (the `counter` of navigation items); `work` is the total.
    pub counts: BTreeMap<String, u64>,
    pub incomplete: Vec<String>,
}

/// Where the caller works.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct WorkQuery {
    /// The organization unit chosen; omitted: the whole tenant.
    org_id: Option<Uuid>,
    /// At most this many items, 1 to 200. Defaults to 50.
    limit: Option<u16>,
    /// `me`: only work assigned to you; `nobody`: only work assigned to no one.
    assigned: Option<String>,
    /// `true`: also the work you snoozed (with `snoozed_until`).
    #[serde(default)]
    include_snoozed: bool,
}

#[derive(Clone)]
pub(crate) struct WorkState {
    pub sources: WorkSources,
    pub entitlements: EntitlementsHandle,
    pub translations: Translations,
    counts: Arc<Mutex<CountCache>>,
}

/// Số đếm đã tính theo (khách, người gọi, nút), kèm lúc tính.
type CountCache = HashMap<(Uuid, Uuid, Uuid), (Instant, WorkCounts)>;

impl WorkState {
    pub(crate) fn new(
        sources: WorkSources,
        entitlements: EntitlementsHandle,
        translations: Translations,
    ) -> Self {
        WorkState { sources, entitlements, translations, counts: Arc::default() }
    }

    /// Nguồn người gọi được thấy ở `target`: module bật cho khách, có permission của nguồn.
    async fn allowed(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
    ) -> Result<Vec<Entry>, Error> {
        let read = tenant.database().read();
        let mut allowed = Vec::new();
        for (module, source) in self.sources.all() {
            if !self.entitlements.module_enabled(tenant.id(), module).await? {
                continue;
            }
            if source.for_any_signed_in()
                || principal.require_at(read, &source.permission(), target).await.is_ok()
            {
                allowed.push((module, source));
            }
        }
        Ok(allowed)
    }

    pub(crate) async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        query: &WorkQuery,
        language: Language,
    ) -> Result<WorkView, Error> {
        let org_id = query.org_id;
        let limit = query.limit.unwrap_or(50);
        let target = target_of(tenant, org_id);
        let sources = self.allowed(tenant, principal, target).await?;
        let answers =
            futures_util::future::join_all(sources.iter().map(|(_, source)| async move {
                let found = tokio::time::timeout(
                    SOURCE_TIMEOUT,
                    source.items(tenant, principal, target, PER_SOURCE),
                )
                .await;
                (source.id(), found)
            }))
            .await;
        let mut items = Vec::new();
        let mut incomplete = Vec::new();
        for (id, answer) in answers {
            match answer {
                Ok(Ok(found)) => {
                    items.extend(found.into_iter().map(|item| self.view(id, item, language)))
                }
                Ok(Err(error)) => {
                    tracing::warn!(source = id, error = %error, "work source failed");
                    incomplete.push(id.to_owned());
                }
                Err(_) => {
                    tracing::warn!(source = id, "work source timed out");
                    incomplete.push(id.to_owned());
                }
            }
        }
        // Gấp trước, rồi hạn gần trước (không hạn xếp sau), rồi theo khoá cho ổn định.
        items.sort_by(|a, b| {
            (a.severity, a.due_at.is_none(), a.due_at, &a.key).cmp(&(
                b.severity,
                b.due_at.is_none(),
                b.due_at,
                &b.key,
            ))
        });
        // Hoãn / giao (P10): đọc một lần cho cả danh sách.
        let keys: Vec<String> = items.iter().map(|item| item.key.clone()).collect();
        let states = crate::ops::work_states::states(tenant, principal.user_id(), &keys).await?;
        let now = Utc::now();
        items.retain_mut(|item| {
            let state = states.get(&item.key).cloned().unwrap_or_default();
            item.assignee_id = state.assignee_id;
            item.snoozed_until = state.snoozed_until.filter(|until| *until > now);
            let shown = query.include_snoozed || item.snoozed_until.is_none();
            let assigned = match query.assigned.as_deref() {
                Some("me") => item.assignee_id == Some(principal.user_id()),
                Some("nobody") => item.assignee_id.is_none(),
                _ => true,
            };
            shown && assigned
        });
        items.truncate(usize::from(limit));
        Ok(WorkView { items, incomplete })
    }

    fn view(&self, source: &str, item: WorkItem, language: Language) -> WorkItemView {
        let template = self.translations.describe(item.title, language);
        let mut title = template;
        for (name, value) in &item.params {
            title = title.replace(&format!("{{{name}}}"), value);
        }
        WorkItemView {
            key: format!("{source}:{}", item.record_id),
            source: source.to_owned(),
            severity: item.severity,
            title,
            due_at: item.due_at,
            target: WorkTarget { screen: item.screen.to_owned(), record_id: item.record_id },
            assignee_id: None,
            snoozed_until: None,
        }
    }

    pub(crate) async fn counts(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Option<Uuid>,
    ) -> Result<WorkCounts, Error> {
        let key = (tenant.id().as_uuid(), principal.user_id(), org_id.unwrap_or(Uuid::nil()));
        {
            let cache = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((at, counts)) = cache.get(&key)
                && at.elapsed() < COUNT_CACHE
            {
                return Ok(counts.clone());
            }
        }
        let target = target_of(tenant, org_id);
        let sources = self.allowed(tenant, principal, target).await?;
        let answers =
            futures_util::future::join_all(sources.iter().map(|(_, source)| async move {
                (
                    source.id(),
                    tokio::time::timeout(SOURCE_TIMEOUT, source.count(tenant, principal, target))
                        .await,
                )
            }))
            .await;
        let mut counts = BTreeMap::new();
        let mut incomplete = Vec::new();
        let mut total = 0;
        for (id, answer) in answers {
            match answer {
                Ok(Ok(count)) => {
                    total += count;
                    counts.insert(id.to_owned(), count);
                }
                _ => incomplete.push(id.to_owned()),
            }
        }
        counts.insert("work".to_owned(), total);
        let result = WorkCounts { counts, incomplete };
        // Kết quả thiếu nguồn (nguồn chậm hay lỗi) không giữ lại: lần hỏi sau thử lại ngay, thay vì
        // hiện số trống suốt `COUNT_CACHE`.
        if result.incomplete.is_empty() {
            let mut cache = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
            // Bộ nhớ không phình mãi: dọn mục đã hết hạn khi thêm mục mới.
            cache.retain(|_, (at, _)| at.elapsed() < COUNT_CACHE);
            cache.insert(key, (Instant::now(), result.clone()));
        }
        Ok(result)
    }
}

fn target_of(tenant: &TenantContext, org_id: Option<Uuid>) -> OrgTarget {
    OrgTarget::new(org_id.map_or(root_of(tenant.id()), OrgId::from_uuid), None)
}

pub(crate) fn routes(state: WorkState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_work))
        .routes(routes!(count_work))
        .with_state(Arc::new(state))
}

/// List the work waiting for you.
///
/// Any signed-in user. Work that modules compute from their data (comments waiting for approval,
/// new form submissions, entries waiting for a review…) from the sources you have permission for
/// at `org_id`, most urgent first. Work disappears when the record is handled; there is no "done"
/// to set. A source that is slow or failing is skipped and named in `incomplete`.
#[utoipa::path(
    get,
    path = "/work",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_work",
    security(("bearer" = [])),
    params(WorkQuery),
    responses(
        (status = 200, description = "The work.", body = Data<WorkView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`limit` is not between 1 and 200, or `assigned` is not `me` or `nobody`; see `errors`.", body = Problem),
    )
)]
async fn list_work(
    State(state): State<Arc<WorkState>>,
    tenant: TenantContext,
    principal: Principal,
    language: Language,
    Query(query): Query<WorkQuery>,
) -> Result<Data<WorkView>, Error> {
    let limit = query.limit.unwrap_or(50);
    let mut problems = Vec::new();
    if !(1..=200).contains(&limit) {
        problems.push(crate::error::FieldError::new(
            "limit",
            "invalid",
            "must be between 1 and 200",
        ));
    }
    if query.assigned.as_deref().is_some_and(|assigned| !["me", "nobody"].contains(&assigned)) {
        problems.push(crate::error::FieldError::new(
            "assigned",
            "invalid",
            "must be `me` or `nobody`",
        ));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok(Data { data: state.items(&tenant, &principal, &query, language).await? })
}

/// Count the work waiting for you, by source.
///
/// Any signed-in user. For the counters of the menu (the `counter` of navigation items): pending
/// work by source at `org_id`, and `work` for the total. Counts are kept for ten seconds.
#[utoipa::path(
    get,
    path = "/work/counts",
    extensions(("x-permissions" = json!([]))),
    operation_id = "count_work",
    security(("bearer" = [])),
    params(WorkQuery),
    responses(
        (status = 200, description = "The counts.", body = Data<WorkCounts>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn count_work(
    State(state): State<Arc<WorkState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<WorkQuery>,
) -> Result<Data<WorkCounts>, Error> {
    Ok(Data { data: state.counts(&tenant, &principal, query.org_id).await? })
}
