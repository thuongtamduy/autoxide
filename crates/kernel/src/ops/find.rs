//! Tìm xuyên phân hệ (bước 51 P3, ô Ctrl K của giao diện): module khai **nguồn tìm** cho loại
//! bản ghi của mình; `GET /api/v1/kernel/find?q=` hỏi các nguồn người gọi có permission, mỗi nguồn
//! có hạn giờ như hộp việc, gom kết quả theo loại. So khớp không dấu (`autoxide_kernel::search`):
//! nguồn tìm trên cột khoá tìm đã lưu sẵn, như mọi ô tìm của module.
//!
//! Lệnh ("tạo mới", "đi tới màn…") giao diện tự dựng từ sổ điều hướng; core không có danh sách lệnh.

use std::sync::{Arc, PoisonError, RwLock};

use axum::extract::{Query, State};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::access::{OrgTarget, Permission, Principal};
use crate::entitlement::EntitlementsHandle;
use crate::error::{Error, FieldError};
use crate::response::{Data, Problem};
use crate::tenancy::{OrgId, TenantContext, root_of};

/// Ít nhất chừng này ký tự (sau khi bỏ khoảng trắng hai đầu).
const MIN_QUERY: usize = 2;

/// Một bản ghi tìm thấy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Found {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Màn của sổ điều hướng mở bản ghi (`NavItem::id`).
    pub screen: &'static str,
}

impl Found {
    pub fn new(id: impl Into<String>, title: impl Into<String>, screen: &'static str) -> Self {
        Found { id: id.into(), title: title.into(), subtitle: None, screen }
    }

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        let subtitle = subtitle.into();
        self.subtitle = (!subtitle.is_empty()).then_some(subtitle);
        self
    }
}

/// Một nguồn tìm module khai bằng [`StartContext::finder_source`](crate::module::StartContext).
/// Nguồn tự áp phạm vi nút của người gọi và chỉ trả bản ghi người đó được xem.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait FinderSource: Send + Sync {
    /// Loại bản ghi, `<module>.<loại>` (như sổ đối tượng), vd `content.entry`.
    fn kind(&self) -> &'static str;
    fn permission(&self) -> Permission;
    async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
        query: &str,
        limit: u16,
    ) -> Result<Vec<Found>, Error>;
}

type Entry = (&'static str, Arc<dyn FinderSource>);

/// Sổ nguồn tìm của mọi module. Clone rẻ.
#[derive(Clone, Default)]
pub struct FinderSources {
    entries: Arc<RwLock<Vec<Entry>>>,
}

impl FinderSources {
    pub(crate) fn register(
        &self,
        module: &'static str,
        source: Arc<dyn FinderSource>,
    ) -> Result<(), Error> {
        let kind = source.kind();
        if !kind.starts_with(&format!("{module}.")) {
            return Err(Error::internal(format!(
                "finder source `{kind}` of module `{module}` must be named `{module}.<type>`"
            )));
        }
        let mut entries = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if entries.iter().any(|(_, known)| known.kind() == kind) {
            return Err(Error::internal(format!("finder source `{kind}` is registered twice")));
        }
        entries.push((module, source));
        Ok(())
    }

    fn all(&self) -> Vec<Entry> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// Records found, by type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FindView {
    /// One group per record type that found something, in the order modules start.
    pub groups: Vec<FoundGroup>,
    /// Sources that did not answer in time or failed.
    pub incomplete: Vec<String>,
}

/// Records of one type.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FoundGroup {
    #[schema(example = "content.entry")]
    pub kind: String,
    pub items: Vec<FoundView>,
}

/// One record found.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct FoundView {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Screen id of the navigation that opens the record.
    #[schema(example = "content.entries")]
    pub screen: String,
}

/// What to find.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct FindQuery {
    /// At least 2 characters; any case, with or without accents.
    q: Option<String>,
    /// The organization unit chosen; omitted: the whole tenant.
    org_id: Option<Uuid>,
    /// At most this many records per type, 1 to 20. Defaults to 5.
    limit: Option<u16>,
}

#[derive(Clone)]
pub(crate) struct FindState {
    pub sources: FinderSources,
    pub entitlements: EntitlementsHandle,
}

impl FindState {
    pub(crate) async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org_id: Option<Uuid>,
        query: &str,
        limit: u16,
    ) -> Result<FindView, Error> {
        let target = OrgTarget::new(org_id.map_or(root_of(tenant.id()), OrgId::from_uuid), None);
        let read = tenant.database().read();
        let mut allowed = Vec::new();
        for (module, source) in self.sources.all() {
            if self.entitlements.module_enabled(tenant.id(), module).await?
                && principal.require_at(read, &source.permission(), target).await.is_ok()
            {
                allowed.push(source);
            }
        }
        let answers = futures_util::future::join_all(allowed.iter().map(|source| async move {
            let found = tokio::time::timeout(
                crate::ops::work::SOURCE_TIMEOUT,
                source.find(tenant, principal, target, query, limit),
            )
            .await;
            (source.kind(), found)
        }))
        .await;
        let mut groups = Vec::new();
        let mut incomplete = Vec::new();
        for (kind, answer) in answers {
            match answer {
                Ok(Ok(found)) if !found.is_empty() => groups.push(FoundGroup {
                    kind: kind.to_owned(),
                    items: found
                        .into_iter()
                        .take(usize::from(limit))
                        .map(|found| FoundView {
                            id: found.id,
                            title: found.title,
                            subtitle: found.subtitle,
                            screen: found.screen.to_owned(),
                        })
                        .collect(),
                }),
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(source = kind, error = %error, "finder source failed");
                    incomplete.push(kind.to_owned());
                }
                Err(_) => {
                    tracing::warn!(source = kind, "finder source timed out");
                    incomplete.push(kind.to_owned());
                }
            }
        }
        Ok(FindView { groups, incomplete })
    }
}

pub(crate) fn routes(state: FindState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(find)).with_state(Arc::new(state))
}

/// Find records across modules.
///
/// Any signed-in user. Asks every module's finder for records whose title (or name, email…)
/// contains `q`, in any case and with or without accents, among the types the caller has
/// permission for at `org_id`; results are grouped by type, each record with the screen that
/// opens it. A slow or failing source is skipped and named in `incomplete`.
#[utoipa::path(
    get,
    path = "/find",
    extensions(("x-permissions" = json!([]))),
    operation_id = "find_records",
    security(("bearer" = [])),
    params(FindQuery),
    responses(
        (status = 200, description = "The records found.", body = Data<FindView>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "`q` is shorter than 2 characters, or `limit` is not between 1 and 20; see `errors`.", body = Problem),
    )
)]
async fn find(
    State(state): State<Arc<FindState>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<FindQuery>,
) -> Result<Data<FindView>, Error> {
    let text = query.q.as_deref().map(str::trim).unwrap_or_default();
    let limit = query.limit.unwrap_or(5);
    let mut problems = Vec::new();
    if text.chars().count() < MIN_QUERY {
        problems.push(
            FieldError::new("q", "too_short", "must be at least {min} characters")
                .with_param("min", MIN_QUERY),
        );
    }
    if !(1..=20).contains(&limit) {
        problems.push(FieldError::new("limit", "invalid", "must be between 1 and 20"));
    }
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    Ok(Data { data: state.find(&tenant, &principal, query.org_id, text, limit).await? })
}
