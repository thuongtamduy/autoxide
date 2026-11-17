//! Số liệu tóm tắt của thư viện media (bước 51 P4).

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::summary::{MetricInfo, MetricValue, SummaryMetric};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::entity::item::{Column, Entity};
use crate::permissions::MEDIA_READ;

/// `media.items`: tệp trong thư viện (ngoài thùng rác), trong phạm vi của người gọi.
pub(crate) struct Items;

#[async_trait::async_trait]
impl SummaryMetric for Items {
    fn info(&self) -> MetricInfo {
        MetricInfo::new(
            "media.items",
            "Media items",
            "Files in the media library you may see, not counting the trash.",
            "content",
        )
    }

    fn permission(&self) -> Permission {
        MEDIA_READ
    }

    async fn value(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
    ) -> Result<MetricValue, Error> {
        let scope =
            principal.scope_condition(&MEDIA_READ, Column::OrgId, Some(Column::UploadedBy))?;
        let count = Entity::find()
            .filter(scope)
            .scoped(tenant)
            .filter(Column::TrashedAt.is_null())
            .count(tenant.database().read())
            .await?;
        Ok(MetricValue::Count(count))
    }
}
