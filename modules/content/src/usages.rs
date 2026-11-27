//! "Đang dùng ở đâu" (bước 51, D-267): bài viết / trang trỏ tới một bản ghi — ảnh đại diện, ảnh
//! chia sẻ, hay ID trong thân bài (ảnh, bản ghi nhúng).

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, Pagination, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::usages::{KeyedReferrer, ReferenceSource, Referrer};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Select};
use uuid::Uuid;

use crate::entity::entry;

pub(crate) struct Entries;

/// ID dùng được trong `LIKE`: chỉ chữ, số và `-` (không có ký tự đại diện của `LIKE`).
fn plain(id: &str) -> bool {
    (1..=100).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Bài trỏ tới `kind` / `id`, kèm khoá media khi `kind` là ảnh; `None` khi `id` không dùng được trong `LIKE`.
fn pointing(
    tenant: &TenantContext,
    kind: &str,
    id: &str,
) -> Option<(Select<entry::Entity>, Option<Uuid>)> {
    if !plain(id) {
        return None;
    }
    let media = if kind == "media.item" { id.parse::<Uuid>().ok() } else { None };
    // Thân bài là JSON trong `text`: ID xuất hiện dạng chuỗi (`mediaId`, `id` của node nhúng).
    // Không có index cho tìm này — chỉ chạy khi người dùng hỏi, theo trang.
    let mut condition = Condition::any().add(entry::Column::Body.like(format!("%{id}%")));
    if let Some(media) = media {
        condition = condition
            .add(entry::Column::FeaturedMediaId.eq(media))
            .add(entry::Column::OgMediaId.eq(media));
    }
    Some((entry::Entity::find().filter(condition).scoped(tenant), media))
}

fn referrer_of(row: entry::Model, media: Option<Uuid>) -> KeyedReferrer {
    let field = match media {
        Some(media) if row.featured_media_id == Some(media) => "featured_media",
        Some(media) if row.og_media_id == Some(media) => "og_media",
        _ => "body",
    };
    KeyedReferrer::new(
        row.id,
        Referrer::new(crate::resource::ENTRY, row.id.to_string(), row.title, field),
    )
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
        let Some((select, media)) = pointing(tenant, kind, id) else { return Ok(Vec::new()) };
        let rows = select
            .order_by_desc(entry::Column::Id)
            .limit(u64::try_from(limit).unwrap_or(u64::MAX))
            .all(tenant.database().read())
            .await?;
        Ok(rows.into_iter().map(|row| referrer_of(row, media).referrer).collect())
    }

    // Keyset theo id của bài (UUIDv7) trong database, cùng con trỏ với các nguồn khác; index (tenant_id, id)
    // có sẵn (m0011).
    async fn referrers_page(
        &self,
        tenant: &TenantContext,
        kind: &str,
        id: &str,
        page: &PageRequest,
    ) -> Result<Page<KeyedReferrer>, Error> {
        let Some((select, media)) = pointing(tenant, kind, id) else {
            let total = page.wants_total().then_some(0);
            return Ok(Page {
                data: Vec::new(),
                pagination: Pagination::new(page.limit(), None).with_total(total),
            });
        };
        let rows =
            fetch_page(select, entry::Column::Id, |row| row.id, page, tenant.database().read())
                .await?;
        Ok(rows.map(|row| referrer_of(row, media)))
    }
}
