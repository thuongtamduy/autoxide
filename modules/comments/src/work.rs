//! Nguồn việc của hộp việc (bước 51 P2): bình luận đang chờ duyệt. Tính từ dữ liệu — duyệt, từ
//! chối hay đánh dấu spam thì việc tự hết. Câu đếm dùng index `(tenant_id, status, id)`.

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::db::Queries as _;
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};

use crate::entity::comment;
use crate::permissions::COMMENTS_MODERATE;

/// `comments.pending`: bình luận chờ duyệt.
pub(crate) struct Pending;

#[autoxide_kernel::async_trait]
impl WorkSource for Pending {
    fn id(&self) -> &'static str {
        "comments.pending"
    }

    fn permission(&self) -> Permission {
        COMMENTS_MODERATE
    }

    async fn count(
        &self,
        tenant: &TenantContext,
        _: &Principal,
        _: OrgTarget,
    ) -> Result<u64, Error> {
        let count = tenant
            .db()
            .read()
            .find::<comment::Model>()
            .filter(comment::Model::STATUS.eq("pending".to_owned()))
            .count()
            .await?;
        Ok(u64::try_from(count).unwrap_or_default())
    }

    async fn items(
        &self,
        tenant: &TenantContext,
        _: &Principal,
        _: OrgTarget,
        limit: u16,
    ) -> Result<Vec<WorkItem>, Error> {
        // Cũ nhất trước: người viết chờ lâu nhất.
        let page = PageRequest::first(u32::from(limit), Order::Asc);
        let rows = tenant
            .db()
            .read()
            .find::<comment::Model>()
            .filter(comment::Model::STATUS.eq("pending".to_owned()))
            .page(comment::Model::ID, &page)
            .await?
            .data;
        Ok(rows
            .into_iter()
            .map(|row| {
                let id = row.id.to_string();
                match row.author_name.filter(|name| !name.trim().is_empty()) {
                    Some(author) => WorkItem::new(
                        id,
                        Severity::Attention,
                        "Approve or reject the comment of {author}",
                        "marketing.comments",
                    )
                    .param("author", author),
                    None => WorkItem::new(
                        id,
                        Severity::Attention,
                        "Approve or reject a comment",
                        "marketing.comments",
                    ),
                }
                .due_at(None)
            })
            .collect())
    }
}

/// Cùng câu đếm, làm số liệu tóm tắt (bước 51 P4).
#[autoxide_kernel::async_trait]
impl autoxide_kernel::summary::SummaryMetric for Pending {
    fn info(&self) -> autoxide_kernel::summary::MetricInfo {
        autoxide_kernel::summary::MetricInfo::new(
            "comments.pending",
            "Comments waiting for approval",
            "Comments of every target with the status `pending` now, whoever wrote them.",
            "marketing",
        )
    }

    fn permission(&self) -> Permission {
        COMMENTS_MODERATE
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
