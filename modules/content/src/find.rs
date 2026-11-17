//! Nguồn tìm xuyên phân hệ (bước 51 P3): bài viết và trang có tiêu đề, tóm tắt hay thân chứa từ
//! tìm (cột `search_key`, không dấu), trong phạm vi của `content.read` của người gọi.

use autoxide_kernel::access::{OrgTarget, Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::find::{FinderSource, Found};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entity::entry::{Column, Entity};
use crate::permissions::CONTENT_READ;

pub(crate) struct Entries;

#[async_trait::async_trait]
impl FinderSource for Entries {
    fn kind(&self) -> &'static str {
        "content.entry"
    }

    fn permission(&self) -> Permission {
        CONTENT_READ
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
            principal.scope_condition(&CONTENT_READ, Column::OrgId, Some(Column::CreatedBy))?;
        let rows = Entity::find()
            .filter(scope)
            .scoped(tenant)
            .filter(autoxide_kernel::search::contains(Column::SearchKey, query))
            .order_by_desc(Column::UpdatedAt)
            .limit(u64::from(limit))
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                Found::new(row.id.to_string(), row.title, "content.entries")
                    .subtitle(format!("{} · {} · {}", row.kind, row.status, row.locale))
            })
            .collect())
    }
}
