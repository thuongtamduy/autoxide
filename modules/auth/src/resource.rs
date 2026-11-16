//! Loại đối tượng `auth.user` trong sổ đối tượng của kernel (D-123): module khác lấy tên hiển
//! thị của người dùng theo lô — vd `audit` trả tên người làm (`include=actor`, đề bài giao diện
//! B2, D-265) — mà không phụ thuộc module `auth` (D-046). Tài khoản luôn riêng tư; chủ của nó
//! là chính người đó.

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use uuid::Uuid;

use crate::user::entity::{Column, Entity};

pub(crate) const USER: &str = "auth.user";

pub(crate) struct UserResolver;

#[async_trait::async_trait]
impl ResourceResolver for UserResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(self.resolve_many(tenant, &[id.to_owned()], None).await?.remove(id))
    }

    /// Một câu truy vấn cho mọi id; id không phải UUID hay không có tài khoản thì vắng.
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
        Ok(rows
            .into_iter()
            .map(|(id, name)| (id.to_string(), Resource::new(name).owner_user_ids(vec![id])))
            .collect())
    }
}
