//! Loại đối tượng của `content` trong sổ đối tượng của kernel (D-123): module khác (bình luận,
//! bản tin, menu) hỏi một bài / trang / danh mục có không, có công khai không, tiêu đề, tóm tắt,
//! đường dẫn — mà không phụ thuộc crate `content` và không đọc bảng của nó.
//!
//! - `content.entry`: bài viết hay trang.
//! - `content.term` (D-238): danh mục hay thẻ; danh mục có cây (`children`). Đường dẫn công khai
//!   `/<tiền tố bài viết>/category/<slug>` hay `/…/tag/<slug>`.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceRef, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::{entry, published, term, term_translation};
use crate::service::ContentService;

/// Tên loại đối tượng của bài viết, trang.
pub(crate) const ENTRY: &str = "content.entry";
/// Tên loại đối tượng của danh mục, thẻ.
pub(crate) const TERM: &str = "content.term";

/// ID hợp lệ (UUID) trong danh sách; ID sai dạng coi như không có.
fn uuids(ids: &[String]) -> Vec<Uuid> {
    let mut found: Vec<Uuid> = ids.iter().filter_map(|id| id.parse().ok()).collect();
    found.sort_unstable();
    found.dedup();
    found
}

// Không cần service: mọi thứ đọc từ database của khách được hỏi.
pub(crate) struct EntryResolver;

#[async_trait::async_trait]
impl ResourceResolver for EntryResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(self.resolve_many(tenant, &[id.to_owned()], None).await?.remove(id))
    }

    // Bài nào cũng có một ngôn ngữ riêng (bản dịch là bài khác), nên `locale` không dùng.
    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        _locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let ids = uuids(ids);
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let read = tenant.database().read();
        let drafts = entry::Entity::find()
            .scoped(tenant)
            .filter(entry::Column::Id.is_in(ids.clone()))
            .all(read)
            .await?;
        let mut live: HashMap<Uuid, published::Model> = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::EntryId.is_in(ids))
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.entry_id, row))
            .collect();
        Ok(drafts
            .into_iter()
            .map(|draft| {
                let owners = draft.created_by.into_iter().collect();
                // Đang công khai: thông tin của bản công khai (thứ người đọc thấy). Không thì
                // `public` là `false` và không đưa tiêu đề, tóm tắt của bản nháp ra ngoài — module
                // khác có thể chép chúng vào mail gửi người lạ (D-150).
                let resource = match live.remove(&draft.id) {
                    Some(live) => Resource::new(live.title)
                        .public(true)
                        .summary(live.excerpt)
                        .url(Some(live.path))
                        .locale(Some(live.locale))
                        .image(live.featured_media_id)
                        .owner_user_ids(owners),
                    None => Resource::new(String::new())
                        .locale(Some(draft.locale))
                        .owner_user_ids(owners),
                };
                (draft.id.to_string(), resource)
            })
            .collect())
    }
}

pub(crate) struct TermResolver(pub Arc<ContentService>);

#[async_trait::async_trait]
impl ResourceResolver for TermResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        Ok(self.resolve_many(tenant, &[id.to_owned()], None).await?.remove(id))
    }

    /// Tên, slug theo `locale`; thiếu thì ngôn ngữ mặc định của `content`, rồi bản dịch bất kỳ.
    /// Danh mục và thẻ luôn công khai (không có bản nháp).
    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let ids = uuids(ids);
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let config = &self.0.config;
        let texts = term_translation::Entity::find()
            .scoped(tenant)
            .filter(term_translation::Column::TermId.is_in(ids))
            .all(tenant.database().read())
            .await?;
        let mut by_term: HashMap<Uuid, Vec<term_translation::Model>> = HashMap::new();
        for text in texts {
            by_term.entry(text.term_id).or_default().push(text);
        }
        let default = self.0.languages(tenant).await?.default;
        let wanted = locale.unwrap_or(&default);
        Ok(by_term
            .into_iter()
            .filter_map(|(id, texts)| {
                let chosen = texts
                    .iter()
                    .find(|text| text.locale == wanted)
                    .or_else(|| texts.iter().find(|text| text.locale == default))
                    .or_else(|| texts.first())?;
                let path =
                    format!("/{}/{}/{}", config.post_path_prefix, chosen.taxonomy, chosen.slug);
                let resource = Resource::new(chosen.name.clone())
                    .public(true)
                    .summary(chosen.description.clone())
                    .url(Some(path))
                    .locale(Some(chosen.locale.clone()));
                Some((id.to_string(), resource))
            })
            .collect())
    }

    fn has_children(&self) -> bool {
        true
    }

    /// Danh mục con trực tiếp, theo thứ tự tạo. Thẻ không có con.
    async fn children(
        &self,
        tenant: &TenantContext,
        id: &str,
        _locale: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ResourceRef>, Error> {
        let Ok(id) = id.parse::<Uuid>() else {
            return Ok(Vec::new());
        };
        let rows = term::Entity::find()
            .scoped(tenant)
            .filter(term::Column::ParentId.eq(id))
            .order_by_asc(term::Column::Id)
            .limit(u64::try_from(limit).unwrap_or(u64::MAX))
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|row| ResourceRef::new(TERM, row.id.to_string())).collect())
    }
}
