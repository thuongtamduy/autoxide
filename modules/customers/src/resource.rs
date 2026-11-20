//! Loại đối tượng `customers.customer` trong sổ đối tượng (D-123): nhãn, ghi chú và trường tự định
//! nghĩa của bước 51 gắn lên hồ sơ qua loại này (D-237). Tiêu đề không mang liên hệ đầy đủ.

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::EntityTrait;

use crate::entity::profile;
use crate::service::mask_phone;

pub(crate) struct CustomerResolver;

#[async_trait::async_trait]
impl ResourceResolver for CustomerResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let Ok(id) = id.parse::<uuid::Uuid>() else {
            return Ok(None);
        };
        let row =
            profile::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?;
        Ok(row.map(|row| {
            let title = row
                .full_name
                .clone()
                .or_else(|| row.phone.as_deref().map(mask_phone))
                .unwrap_or_else(|| "Customer".to_owned());
            Resource::new(title).url(Some(format!("/customers/{id}")))
        }))
    }
}
