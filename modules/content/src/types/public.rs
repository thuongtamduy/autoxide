//! Đọc công khai mục của loại tự định nghĩa (D-235): chỉ loại đang bật và `listable`, chỉ bản đã đăng
//! của website đang xem, mới nhất trước.

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};

use super::{ContentType, type_not_found};
use crate::entity::{content_type, published};
use crate::publishing::Published;
use crate::publishing::reading::on_site;
use crate::service::ContentService;

/// Mục `kind = custom` chỉ công khai khi loại của nó đang bật (`subquery`, như nhau trên hai database).
pub(crate) fn type_is_active(tenant: &TenantContext) -> Condition {
    let active = Query::select()
        .column(content_type::Column::Id)
        .from(content_type::Entity)
        .and_where(Expr::col(content_type::Column::TenantId).eq(tenant.id().as_uuid()))
        .and_where(Expr::col(content_type::Column::Active).eq(true))
        .to_owned();
    Condition::any()
        .add(published::Column::TypeId.is_null())
        .add(published::Column::TypeId.in_subquery(active))
}

impl ContentService {
    /// Mục đã đăng của loại `key` ở ngôn ngữ `locale` trên website `site`. Loại không có, đang tắt
    /// hay không `listable` → 404 `content.type_not_found` (không lộ loại tồn tại).
    pub(crate) async fn public_type_entries(
        &self,
        tenant: &TenantContext,
        site: &Site,
        key: &str,
        locale: &str,
        page: &PageRequest,
    ) -> Result<(ContentType, Page<Published>), Error> {
        let found = self.find_type(tenant, key).await.ok().filter(|row| row.active && row.listable);
        let Some(row) = found else { return Err(type_not_found(key)) };
        let select = published::Entity::find()
            .scoped(tenant)
            .filter(published::Column::RestrictedAt.is_null())
            .filter(published::Column::TypeId.eq(row.id))
            .filter(published::Column::Locale.eq(locale))
            .filter(on_site(&site.lineage_ids()));
        let sort = |entry: &Published| SortValue::Time(entry.published_at);
        let read = tenant.database().read();
        let page = fetch_page_by(
            select,
            published::Column::PublishedAt,
            sort,
            published::Column::EntryId,
            |entry| entry.entry_id,
            page,
            read,
        )
        .await?;
        Ok((row, page))
    }
}
