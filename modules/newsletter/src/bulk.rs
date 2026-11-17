//! Thao tác hàng loạt trên danh sách người đăng ký (bước 51 P8): chặn, bỏ chặn, xoá — cùng hàm,
//! cùng kiểm quyền với `PATCH` / `DELETE /newsletter/subscribers/{id}`, không kiểm phiên bản. Người
//! đã ở trạng thái đích thì bỏ qua, nên job chạy lại không làm hai lần.

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::bulk::{BulkAction, BulkInfo, BulkOutcome};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::TenantContext;
use uuid::Uuid;

use crate::audience::subscribers::{SubscriberChange, SubscriberStatus};
use crate::permissions::NEWSLETTER_MANAGE;
use crate::service::NewsletterService;

const LIST: &str = "newsletter.list_newsletter_subscribers";

/// Ba hành động của danh sách người đăng ký.
pub(crate) fn actions(service: &Arc<NewsletterService>) -> Vec<Arc<dyn BulkAction>> {
    vec![
        Arc::new(SetStatus {
            service: service.clone(),
            to: SubscriberStatus::Blocked,
            action: "block",
            label: "Block subscribers",
        }),
        Arc::new(SetStatus {
            service: service.clone(),
            to: SubscriberStatus::Active,
            action: "unblock",
            label: "Unblock subscribers",
        }),
        Arc::new(Delete(service.clone())),
    ]
}

fn parse(id: &str) -> Result<Uuid, Error> {
    // Id không phải UUID thì như không có người đăng ký nào mang id đó.
    id.parse().map_err(|_| crate::service::subscriber_not_found())
}

pub(crate) struct SetStatus {
    service: Arc<NewsletterService>,
    to: SubscriberStatus,
    action: &'static str,
    label: &'static str,
}

#[async_trait::async_trait]
impl BulkAction for SetStatus {
    fn info(&self) -> BulkInfo {
        BulkInfo::new(LIST, self.action, self.label)
    }

    fn permission(&self) -> Permission {
        NEWSLETTER_MANAGE
    }

    async fn apply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<BulkOutcome, Error> {
        let id = parse(id)?;
        principal.require(&NEWSLETTER_MANAGE)?;
        let found = self.service.get_subscriber(tenant, principal, id).await?;
        if found.row.status == self.to.as_str() {
            return Ok(BulkOutcome::Skipped("unchanged"));
        }
        let change = SubscriberChange {
            name: None,
            locale: None,
            status: Some(self.to),
            blocked_reason: None,
            attributes: None,
            unsubscribe_list_ids: Vec::new(),
        };
        self.service.update_subscriber(tenant, principal, id, change, None).await?;
        Ok(BulkOutcome::Done)
    }
}

pub(crate) struct Delete(Arc<NewsletterService>);

#[async_trait::async_trait]
impl BulkAction for Delete {
    fn info(&self) -> BulkInfo {
        BulkInfo::new(LIST, "delete", "Delete subscribers")
    }

    fn permission(&self) -> Permission {
        NEWSLETTER_MANAGE
    }

    async fn apply(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: &str,
    ) -> Result<BulkOutcome, Error> {
        self.0.delete_subscriber(tenant, principal, parse(id)?).await?;
        Ok(BulkOutcome::Done)
    }
}
