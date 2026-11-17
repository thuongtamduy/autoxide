//! Nguồn việc của hộp việc (bước 51 P2): bình luận đang chờ duyệt. Tính từ dữ liệu — duyệt, từ
//! chối hay đánh dấu spam thì việc tự hết. Câu đếm dùng index `(tenant_id, status, id)`.

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Order, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::work::{Severity, WorkItem, WorkSource};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::entity::comment;
use crate::permissions::COMMENTS_MODERATE;

/// `comments.pending`: bình luận chờ duyệt.
pub(crate) struct Pending;

#[async_trait::async_trait]
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
        Ok(comment::Entity::find()
            .scoped(tenant)
            .filter(comment::Column::Status.eq("pending"))
            .count(tenant.database().read())
            .await?)
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
        let select =
            comment::Entity::find().scoped(tenant).filter(comment::Column::Status.eq("pending"));
        let rows =
            fetch_page(select, comment::Column::Id, |row| row.id, &page, tenant.database().read())
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
