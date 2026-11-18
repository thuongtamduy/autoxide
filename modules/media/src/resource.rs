//! Tệp media trong sổ đối tượng (loại `media.item`): để kernel kiểm "bản ghi có không" khi hỏi
//! "đang dùng ở đâu" (bước 51, D-267) và để module khác nhắc tới tệp. Tệp công khai: tiêu đề là tên
//! tệp, ảnh đại diện là chính nó; tệp riêng tư không đưa tên ra ngoài. Tệp trong thùng rác coi như
//! không có.

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::item::{Column, Entity};

pub(crate) const ITEM: &str = "media.item";

pub(crate) struct ItemResolver;

#[async_trait::async_trait]
impl ResourceResolver for ItemResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(self.resolve_many(tenant, &[id.to_owned()], None).await?.remove(id))
    }

    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        _locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let ids: Vec<Uuid> = ids.iter().filter_map(|id| id.parse().ok()).collect();
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = Entity::find()
            .filter(Column::Id.is_in(ids))
            .filter(Column::TrashedAt.is_null())
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let owners = row.uploaded_by.into_iter().collect();
                let resource = if row.visibility == "public" {
                    Resource::new(row.name).public(true).image(Some(row.id)).owner_user_ids(owners)
                } else {
                    Resource::new(String::new()).owner_user_ids(owners)
                };
                (row.id.to_string(), resource)
            })
            .collect())
    }
}
