//! Số liệu tóm tắt của bản tin (bước 51 P4; đề bài giao diện: tổng số đăng ký và số địa chỉ không
//! gửi của cả khách). Câu đếm dùng index `(tenant_id, id)` của hai bảng.

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::summary::{MetricInfo, MetricValue, SummaryMetric};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::entity::{subscriber, suppression};
use crate::permissions::NEWSLETTER_READ;

/// `newsletter.subscribers`: người đăng ký không bị chặn.
pub(crate) struct Subscribers;

#[async_trait::async_trait]
impl SummaryMetric for Subscribers {
    fn info(&self) -> MetricInfo {
        MetricInfo::new(
            "newsletter.subscribers",
            "Newsletter subscribers",
            "People in the newsletter's address book who are not blocked, whatever lists they are in and whether they confirmed.",
            "marketing",
        )
    }

    fn permission(&self) -> Permission {
        NEWSLETTER_READ
    }

    async fn value(
        &self,
        tenant: &TenantContext,
        _: &Principal,
        _: OrgTarget,
    ) -> Result<MetricValue, Error> {
        let count = subscriber::Entity::find()
            .scoped(tenant)
            .filter(subscriber::Column::Status.ne("blocked"))
            .count(tenant.database().read())
            .await?;
        Ok(MetricValue::Count(count))
    }
}

/// `newsletter.suppressed`: địa chỉ không bao giờ gửi (huỷ, dội, khiếu nại, chặn tay).
pub(crate) struct Suppressed;

#[async_trait::async_trait]
impl SummaryMetric for Suppressed {
    fn info(&self) -> MetricInfo {
        MetricInfo::new(
            "newsletter.suppressed",
            "Addresses never mailed",
            "Addresses on the do-not-mail list: unsubscribed everywhere, bounced, complained or added by hand.",
            "marketing",
        )
    }

    fn permission(&self) -> Permission {
        NEWSLETTER_READ
    }

    async fn value(
        &self,
        tenant: &TenantContext,
        _: &Principal,
        _: OrgTarget,
    ) -> Result<MetricValue, Error> {
        Ok(MetricValue::Count(
            suppression::Entity::find().scoped(tenant).count(tenant.database().read()).await?,
        ))
    }
}
