//! Loại đối tượng `content.entry` trong sổ đối tượng của kernel (D-117): module khác (bình luận,
//! bản tin) hỏi một bài / trang có không, có công khai không, tiêu đề, tóm tắt, đường dẫn — mà
//! không phụ thuộc crate `content` và không đọc bảng của nó.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceResolver};
use sea_orm::EntityTrait;
use uuid::Uuid;

use crate::entity::{entry, published};
use crate::service::ContentService;

/// Tên loại đối tượng.
pub(crate) const ENTRY: &str = "content.entry";

pub(crate) struct EntryResolver(pub Arc<ContentService>);

#[async_trait::async_trait]
impl ResourceResolver for EntryResolver {
    async fn resolve(&self, id: &str) -> Result<Option<Resource>, Error> {
        let Ok(id) = id.parse::<Uuid>() else {
            return Ok(None);
        };
        let read = self.0.database.read();
        let Some(draft) = entry::Entity::find_by_id(id).one(read).await? else {
            return Ok(None);
        };
        let owners = draft.created_by.into_iter().collect();
        // Đang công khai: thông tin của bản công khai (thứ người đọc thấy). Không thì của bản nháp,
        // và `public` là `false`.
        Ok(Some(match published::Entity::find_by_id(id).one(read).await? {
            Some(live) => Resource {
                public: true,
                title: live.title,
                summary: live.excerpt,
                url: Some(live.path),
                locale: Some(live.locale),
                owner_user_ids: owners,
            },
            None => Resource {
                public: false,
                title: draft.title,
                summary: draft.excerpt,
                url: None,
                locale: Some(draft.locale),
                owner_user_ids: owners,
            },
        }))
    }
}
