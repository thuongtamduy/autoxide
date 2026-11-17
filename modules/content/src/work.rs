//! Nguồn việc của hộp việc (bước 51 P2): bài, trang đang chờ duyệt (`review_requested_at`), trong
//! phạm vi nút của `content.publish` của người gọi. Đăng hay huỷ yêu cầu duyệt thì việc tự hết.
//! Câu đếm dùng index `(tenant_id, review_requested_at, id)` (migration m0015).

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entity::entry::{Column, Entity};
use crate::permissions::CONTENT_PUBLISH;

/// `content.review`: mục chờ duyệt.
pub(crate) struct Review;

fn waiting(
    tenant: &TenantContext,
    principal: &Principal,
) -> Result<sea_orm::Select<Entity>, Error> {
    let scope = principal.scope_condition(&CONTENT_PUBLISH, Column::OrgId, None)?;
    Ok(Entity::find().filter(scope).scoped(tenant).filter(Column::ReviewRequestedAt.is_not_null()))
}

#[async_trait::async_trait]
impl WorkSource for Review {
    fn id(&self) -> &'static str {
        "content.review"
    }

    fn permission(&self) -> Permission {
        CONTENT_PUBLISH
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        Ok(waiting(tenant, principal)?.count(tenant.database().read()).await?)
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        let rows = waiting(tenant, principal)?
            .order_by_asc(Column::ReviewRequestedAt)
            .order_by_asc(Column::Id)
            .limit(u64::from(limit))
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                WorkItem::new(
                    row.id.to_string(),
                    Severity::Attention,
                    "Review and publish “{title}”",
                    "content.entries",
                )
                .param("title", row.title)
            })
            .collect())
    }
}

/// Cùng câu đếm, làm số liệu tóm tắt (bước 51 P4).
#[async_trait::async_trait]
impl autoxide_kernel::summary::SummaryMetric for Review {
    fn info(&self) -> autoxide_kernel::summary::MetricInfo {
        autoxide_kernel::summary::MetricInfo::new(
            "content.review",
            "Entries waiting for a review",
            "Posts and pages whose writer asked for a review that was not published or withdrawn yet, in your scope.",
            "content",
        )
    }

    fn permission(&self) -> Permission {
        CONTENT_PUBLISH
    }

    async fn value(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: OrgTarget,
    ) -> Result<autoxide_kernel::summary::MetricValue, Error> {
        Ok(autoxide_kernel::summary::MetricValue::Count(
            WorkSource::count(self, tenant, principal, target).await?,
        ))
    }
}
