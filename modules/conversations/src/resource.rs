//! Bộ tra loại đối tượng `conversations.thread` cho sổ đối tượng (D-123): phiếu hỗ trợ chuyển từ
//! chat liên kết về cuộc trò chuyện. Không công khai, không lộ nội dung.

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::EntityTrait;

pub(crate) struct ThreadResolver;

#[async_trait::async_trait]
impl ResourceResolver for ThreadResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let Ok(id) = id.parse::<uuid::Uuid>() else {
            return Ok(None);
        };
        let row = crate::entity::thread::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        Ok(row.map(|_| {
            Resource::new("Customer conversation".to_owned())
                .url(Some(format!("/conversations/threads/{id}")))
        }))
    }
}
