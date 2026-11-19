//! Nguồn gợi ý tìm của `catalog` (D-201) cho `GET /public/search/suggest`: sản phẩm đang bán ở
//! website và danh mục đang hiện. Hai tầng: khớp đúng dấu trên `name_key` trước, khớp không dấu
//! trên `search_key` (kèm từ đồng nghĩa của khách) sau.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::search;
use autoxide_kernel::suggest::{SuggestSource, Suggestion};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entity::{storefront_entry, translation};
use crate::service::CatalogService;

/// Sản phẩm đang bán ở website.
pub(crate) struct ProductSuggestions(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl SuggestSource for ProductSuggestions {
    fn kind(&self) -> &'static str {
        "catalog.product"
    }

    async fn suggest(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        q: &str,
        limit: u16,
    ) -> Result<Vec<Suggestion>, Error> {
        let service = &self.0;
        let read = tenant.database().read();
        let visible = Query::select()
            .column(storefront_entry::Column::ProductId)
            .from(storefront_entry::Entity)
            .and_where(storefront_entry::Column::TenantId.eq(tenant.id().as_uuid()))
            .cond_where(service.visible_on(tenant, site).await?)
            .to_owned();
        let base = || {
            translation::Entity::find()
                .filter(translation::Column::Locale.eq(locale))
                .filter(translation::Column::ProductId.in_subquery(visible.clone()))
                .scoped(tenant)
                .order_by_asc(translation::Column::NameKey)
                .limit(u64::from(limit))
        };
        let exact = base()
            .filter(search::contains_exact(translation::Column::NameKey, q))
            .all(read)
            .await?;
        let mut loose_condition = Condition::any();
        for variant in service.expand_query(tenant, q).await? {
            loose_condition =
                loose_condition.add(search::contains(translation::Column::SearchKey, &variant));
        }
        let loose = base().filter(loose_condition).all(read).await?;
        let prefix = &service.config.product_path_prefix;
        let mut out: Vec<Suggestion> = Vec::new();
        for (rows, is_exact) in [(exact, true), (loose, false)] {
            for row in rows {
                if out.iter().any(|item| item.id == Some(row.product_id)) {
                    continue;
                }
                out.push(
                    Suggestion::new("catalog.product", row.name, is_exact)
                        .id(row.product_id)
                        .path(format!("/{prefix}/{}", row.slug)),
                );
            }
        }
        out.truncate(usize::from(limit));
        Ok(out)
    }
}

/// Danh mục đang hiện (cây danh mục nhỏ: lọc trong ứng dụng). `context` là tên danh mục cha;
/// website mở danh mục theo `id` (`GET /public/categories`).
pub(crate) struct CategorySuggestions(pub(crate) Arc<CatalogService>);

#[async_trait::async_trait]
impl SuggestSource for CategorySuggestions {
    fn kind(&self) -> &'static str {
        "catalog.category"
    }

    async fn suggest(
        &self,
        tenant: &TenantContext,
        _site: &Site,
        locale: &str,
        q: &str,
        limit: u16,
    ) -> Result<Vec<Suggestion>, Error> {
        let categories = self.0.public_categories(tenant, locale).await?;
        let wanted_exact = search::exact_key(q);
        let wanted = search::fold(q);
        let mut exact = Vec::new();
        let mut loose = Vec::new();
        for row in &categories {
            let parent = row
                .category
                .parent_id
                .and_then(|parent| categories.iter().find(|other| other.category.id == parent))
                .map(|parent| parent.text.name.clone())
                .unwrap_or_default();
            let item = |is_exact: bool| {
                Suggestion::new("catalog.category", row.text.name.clone(), is_exact)
                    .id(row.category.id)
                    .context(parent.clone())
            };
            if search::exact_key(&row.text.name).contains(&wanted_exact) {
                exact.push(item(true));
            } else if search::fold(&row.text.name).contains(&wanted) {
                loose.push(item(false));
            }
        }
        exact.extend(loose);
        exact.truncate(usize::from(limit));
        Ok(exact)
    }
}
