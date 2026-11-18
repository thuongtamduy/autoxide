//! "Đang dùng ở đâu" (bước 51, D-267): bài viết / trang trỏ tới một bản ghi — ảnh đại diện, ảnh
//! chia sẻ, hay ID trong thân bài (ảnh, bản ghi nhúng).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::usages::{ReferenceSource, Referrer};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use uuid::Uuid;

use crate::entity::entry;

pub(crate) struct Entries;

/// ID dùng được trong `LIKE`: chỉ chữ, số và `-` (không có ký tự đại diện của `LIKE`).
fn plain(id: &str) -> bool {
    (1..=100).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[async_trait::async_trait]
impl ReferenceSource for Entries {
    async fn referrers(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        limit: usize,
    ) -> Result<Vec<Referrer>, Error> {
        if !plain(id) {
            return Ok(Vec::new());
        }
        let media = if kind == "media.item" { id.parse::<Uuid>().ok() } else { None };
        // Thân bài là JSON trong `text`: ID xuất hiện dạng chuỗi (`mediaId`, `id` của node nhúng).
        // Không có index cho tìm này — chỉ chạy khi người dùng hỏi, có giới hạn.
        let mut condition = Condition::any().add(entry::Column::Body.like(format!("%{id}%")));
        if let Some(media) = media {
            condition = condition
                .add(entry::Column::FeaturedMediaId.eq(media))
                .add(entry::Column::OgMediaId.eq(media));
        }
        let rows = entry::Entity::find()
            .filter(condition)
            .scoped(tenant)
            .order_by_desc(entry::Column::UpdatedAt)
            .limit(u64::try_from(limit).unwrap_or(u64::MAX))
            .all(tenant.database().read())
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let field = match media {
                    Some(media) if row.featured_media_id == Some(media) => "featured_media",
                    Some(media) if row.og_media_id == Some(media) => "og_media",
                    _ => "body",
                };
                Referrer::new(crate::resource::ENTRY, row.id.to_string(), row.title, field)
            })
            .collect())
    }
}
