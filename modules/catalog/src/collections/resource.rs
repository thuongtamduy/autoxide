//! Loại `catalog.collection` trong sổ đối tượng của kernel (D-123, D-238): banner, menu trỏ tới bộ
//! sưu tập. Công khai khi `active` trong khung giờ và có chữ ở ngôn ngữ hỏi; đường dẫn
//! `/collections/<slug>`. Bộ sưu tập chưa công khai không đưa tiêu đề ra ngoài module.

use autoxide_kernel::time::Timestamp;
use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::public::shown;
use crate::entity::{collection, collection_text};

pub(crate) const COLLECTION: &str = "catalog.collection";

/// Đầu đường dẫn công khai của bộ sưu tập.
pub(crate) const PATH_PREFIX: &str = "collections";

pub(crate) struct CollectionResolver;

#[async_trait::async_trait]
impl ResourceResolver for CollectionResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let mut found = self.resolve_many(tenant, &[id.to_owned()], None).await?;
        Ok(found.remove(id))
    }

    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let wanted: Vec<Uuid> = ids.iter().filter_map(|id| id.parse().ok()).collect();
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let read = tenant.database().read();
        let rows = collection::Entity::find()
            .filter(collection::Column::Id.is_in(wanted.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        let texts = collection_text::Entity::find()
            .filter(collection_text::Column::CollectionId.is_in(wanted))
            .scoped(tenant)
            .all(read)
            .await?;
        let now = Timestamp::now();
        Ok(rows
            .into_iter()
            .map(|row| {
                let own: Vec<&collection_text::Model> =
                    texts.iter().filter(|text| text.collection_id == row.id).collect();
                // Ngôn ngữ hỏi; không hỏi thì bản đầu tiên.
                let text = match locale {
                    Some(locale) => own.iter().find(|text| text.locale == locale).copied(),
                    None => own.first().copied(),
                };
                let public = shown(&row, now) && text.is_some();
                let resource = match text.filter(|_| public) {
                    Some(text) => Resource::new(text.title.clone())
                        .public(true)
                        .summary(text.meta_description.clone())
                        .url(Some(format!("/{PATH_PREFIX}/{}", text.slug)))
                        .locale(Some(text.locale.clone()))
                        .image(row.media_id),
                    None => Resource::new(String::new()),
                };
                (row.id.to_string(), resource)
            })
            .collect())
    }
}
