//! Nguồn tìm xuyên phân hệ (bước 51 P3): tệp trong thư viện media (ngoài thùng rác) có tên chứa từ
//! tìm (cột `name_key`, không dấu), trong phạm vi của `media.read` của người gọi.

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::find::{FinderSource, Found};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entity::item::{Column, Entity};
use crate::permissions::MEDIA_READ;

pub(crate) struct Items;

#[async_trait::async_trait]
impl FinderSource for Items {
    fn kind(&self) -> &'static str {
        "media.item"
    }

    fn permission(&self) -> Permission {
        MEDIA_READ
    }

    async fn find(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        _: OrgTarget,
        query: &str,
        limit: u16,
    ) -> Result<Vec<Found>, Error> {
        let scope =
            principal.scope_condition(&MEDIA_READ, Column::OrgId, Some(Column::UploadedBy))?;
        let rows = Entity::find()
            .filter(scope)
            .scoped(tenant)
            .filter(Column::TrashedAt.is_null())
            .filter(autoxide_kernel::search::contains(Column::NameKey, query))
            .order_by_desc(Column::Id)
            .limit(u64::from(limit))
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                Found::new(row.id.to_string(), row.name, "content.media").subtitle(row.content_type)
            })
            .collect())
    }
}
