//! Loại đối tượng `organizations.unit` trong sổ đối tượng của kernel (D-123, D-383): module khác lấy
//! tên của nút tổ chức theo lô — vd `compliance` hiện tên người bán trên hồ sơ định danh trước khi
//! xác minh — mà không phụ thuộc module `organizations`. Nút luôn riêng tư (`public = false`): hồ sơ
//! công khai của cửa hàng là loại khác (`organizations.store`).

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::entity::unit::{Column, Entity};

pub(crate) const UNIT: &str = "organizations.unit";

pub(crate) struct UnitResolver;

#[async_trait::async_trait]
impl ResourceResolver for UnitResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(self.resolve_many(tenant, &[id.to_owned()], None).await?.remove(id))
    }

    /// Một câu truy vấn cho mọi id; id không phải UUID hay không có nút thì vắng.
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
        let rows: Vec<(Uuid, String)> = Entity::find()
            .select_only()
            .column(Column::Id)
            .column(Column::Name)
            .filter(Column::Id.is_in(ids))
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|(id, name)| (id.to_string(), Resource::new(name))).collect())
    }
}
