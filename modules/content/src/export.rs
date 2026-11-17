//! Xuất danh sách bài viết và trang theo bộ lọc (bước 51 P16): cùng bộ lọc, cùng cách đọc, cùng
//! phạm vi quyền với `GET /api/v1/content/entries` — job của kernel gọi từng trang 200 bài.

use std::sync::Arc;

use autoxide_kernel::access::{Permission, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::list_exports::{ExportInfo, ExportPage, ExportQuery, ExportSource};
use autoxide_kernel::pagination::PageQuery;
use autoxide_kernel::tenancy::TenantContext;

use crate::http::entries::{EntriesQuery, describe};
use crate::http::page_request;
use crate::permissions::CONTENT_READ;
use crate::service::ContentService;

const COLUMNS: [&str; 12] = [
    "id",
    "kind",
    "locale",
    "status",
    "title",
    "slug",
    "excerpt",
    "parent_id",
    "word_count",
    "created_at",
    "updated_at",
    "published_at",
];

pub(crate) struct Entries(pub(crate) Arc<ContentService>);

#[async_trait::async_trait]
impl ExportSource for Entries {
    fn info(&self) -> ExportInfo {
        ExportInfo::new("content.list_content_entries", "content-entries", COLUMNS.to_vec())
    }

    fn permission(&self) -> Permission {
        CONTENT_READ
    }

    async fn page(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        query: &ExportQuery,
        cursor: Option<String>,
    ) -> Result<ExportPage, Error> {
        let mut query = query.with("limit", "200");
        if let Some(cursor) = cursor {
            query = query.with("cursor", cursor);
        }
        let filters: EntriesQuery = query.parse()?;
        let paging: PageQuery = query.parse()?;
        let mut problems = Vec::new();
        let filter = filters.filter(principal.user_id(), &mut problems);
        let page = page_request(paging, problems)?;
        let page = self.0.list(tenant, principal, &filter, &page).await?;
        let rows = describe(&self.0, tenant, page.data).await?;
        ExportPage::from_json(&rows, &COLUMNS, page.pagination.next_cursor)
    }
}
