//! Thao tác hàng loạt trên danh sách bình luận (bước 51 P8): duyệt, từ chối, đánh dấu spam, xoá
//! hẳn — cùng hàm, cùng kiểm quyền với `PATCH /comments/{id}` và `DELETE /comments/{id}`, không
//! kiểm phiên bản (người kiểm duyệt đã chọn từ danh sách). Bình luận đã ở trạng thái đích thì bỏ
//! qua, nên job chạy lại không làm hai lần.

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::bulk::{BulkAction, BulkInfo, BulkOutcome};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

use crate::permissions::COMMENTS_MODERATE;
use crate::service::{CommentsService, Status, comment_not_found};

const LIST: &str = "comments.list_comments";

/// Đổi trạng thái hàng loạt sang `to`.
pub(crate) struct Moderate {
    pub service: Arc<CommentsService>,
    pub to: Status,
    pub action: &'static str,
    pub label: &'static str,
}

/// Bốn hành động của danh sách bình luận.
pub(crate) fn actions(service: &Arc<CommentsService>) -> Vec<Arc<dyn BulkAction>> {
    let moderate = |to, action, label| -> Arc<dyn BulkAction> {
        Arc::new(Moderate { service: service.clone(), to, action, label })
    };
    vec![
        moderate(Status::Published, "approve", "Approve comments"),
        moderate(Status::Rejected, "reject", "Reject comments"),
        moderate(Status::Spam, "spam", "Mark comments as spam"),
        Arc::new(Delete(service.clone())),
    ]
}

fn parse(id: &str) -> Result<Uuid, Error> {
    // Id không phải UUID thì như không có bình luận nào mang id đó.
    id.parse().map_err(|_| comment_not_found(Uuid::nil()))
}

#[async_trait::async_trait]
impl BulkAction for Moderate {
    fn info(&self) -> BulkInfo {
        BulkInfo::new(LIST, self.action, self.label)
    }

    fn permission(&self) -> Permission {
        COMMENTS_MODERATE
    }

    async fn apply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<BulkOutcome, Error> {
        let id = parse(id)?;
        principal.require(&COMMENTS_MODERATE)?;
        if Status::of(&self.service.find(tenant, id).await?) == self.to {
            return Ok(BulkOutcome::Skipped("unchanged"));
        }
        self.service.set_status(tenant, principal, id, self.to, None).await?;
        Ok(BulkOutcome::Done)
    }
}

/// Xoá hẳn hàng loạt.
pub(crate) struct Delete(Arc<CommentsService>);

#[async_trait::async_trait]
impl BulkAction for Delete {
    fn info(&self) -> BulkInfo {
        BulkInfo::new(LIST, "delete", "Delete comments for good")
    }

    fn permission(&self) -> Permission {
        COMMENTS_MODERATE
    }

    async fn apply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<BulkOutcome, Error> {
        self.0.hard_delete(tenant, principal, parse(id)?).await?;
        Ok(BulkOutcome::Done)
    }
}
