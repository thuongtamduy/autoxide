//! Nguồn gợi ý tìm của `content` (D-201) cho `GET /api/v1/catalog/public/search/suggest`: bài viết
//! và trang đang công khai ở website, đúng ngôn ngữ. Đọc như trang công khai: bản đã đăng, không bị
//! hạn chế (D-363), không `noindex`, hiện trên website (`on_site`, D-307).
//!
//! Lọc bằng chỉ mục từ của bản công khai (`publishing::words`: tiêu đề, tóm tắt và thân, không dấu), rồi xếp hai tầng
//! trong ứng dụng: tiêu đề khớp đúng dấu trước, còn lại sau (D-202).

use autoxide_kernel::error::Error;
use autoxide_kernel::search;
use autoxide_kernel::suggest::{SuggestSource, Suggestion};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entity::published;
use crate::publishing::reading::on_site;
use crate::publishing::words;

/// Bài viết và trang đang công khai.
pub(crate) struct Entries;

#[async_trait::async_trait]
impl SuggestSource for Entries {
    fn kind(&self) -> &'static str {
        "content.entry"
    }

    async fn suggest(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        q: &str,
        limit: u16,
    ) -> Result<Vec<Suggestion>, Error> {
        // Lấy dư một ít để tầng "khớp đúng dấu" có chỗ chen lên trước khi cắt còn `limit`.
        let fetch = u64::from(limit).saturating_mul(2);
        let read = tenant.database().read();
        // Chỉ mục từ (PT-15, D-20261126-content-word-index) thay `LIKE '%q%'` trên mọi bài.
        let matching = words::Matching { tenant: tenant.id().as_uuid(), locale, kind: None };
        let ids = words::matching_ids(read, matching, &words::WordQuery::parse(q)).await?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::RestrictedAt.is_null())
            .filter(published::Column::Noindex.eq(false))
            .filter(published::Column::Locale.eq(locale))
            .filter(on_site(&site.lineage_ids()))
            .filter(published::Column::EntryId.is_in(ids))
            .order_by_desc(published::Column::PublishedAt)
            .limit(fetch)
            .all(read)
            .await?;
        Ok(rank(rows, q, limit))
    }
}

/// Xếp hai tầng: tiêu đề chứa từ tìm đúng dấu trước; giữ thứ tự mới nhất trong mỗi tầng.
fn rank(rows: Vec<published::Model>, q: &str, limit: u16) -> Vec<Suggestion> {
    let wanted = search::exact_key(q);
    let (exact, loose): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|row| search::exact_key(&row.title).contains(&wanted));
    exact
        .into_iter()
        .map(|row| (row, true))
        .chain(loose.into_iter().map(|row| (row, false)))
        .take(usize::from(limit))
        .map(|(row, is_exact)| {
            Suggestion::new("content.entry", row.title, is_exact).id(row.entry_id).path(row.path)
        })
        .collect()
}
