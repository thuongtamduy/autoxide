//! Trang công khai của bộ sưu tập: chỉ bộ sưu tập `active` trong khung giờ, có chữ ở ngôn ngữ hỏi;
//! sản phẩm cùng hình dạng thẻ như `GET /public/products`, lọc theo website như danh sách đó.
//!
//! - Quy tắc → bộ lọc của danh sách công khai (cùng index của `catalog_storefront_entries`).
//! - Chọn tay, sắp `manual`: phân trang trên `catalog_collection_items` theo `position` (rồi ID sản
//!   phẩm), chỉ sản phẩm còn hiện trên website (subquery của bộ lọc công khai).
//! - Chọn tay, sắp kiểu khác: bộ lọc công khai cộng điều kiện "thuộc bộ sưu tập".

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use super::rules::Rules;
use crate::entity::{collection, collection_item, collection_text, storefront_entry};
use crate::public::{PublicCard, PublicFilter, PublicSort};
use crate::public_filter::Skip;
use crate::service::CatalogService;

/// Bộ sưu tập công khai và một trang sản phẩm của nó.
#[derive(Debug, Clone)]
pub(crate) struct PublicCollection {
    pub collection: collection::Model,
    pub text: collection_text::Model,
    pub products: Page<PublicCard>,
}

/// Kiểu sắp `sort` đi theo thứ tự tăng dần (thứ tự chọn tay, giá tăng); còn lại giảm dần.
pub(crate) fn ascending(sort: &str) -> bool {
    matches!(sort, "manual" | "price_asc")
}

/// Bộ sưu tập có hiện lúc `now` không.
pub(crate) fn shown(row: &collection::Model, now: DateTime<Utc>) -> bool {
    row.status == "active"
        && row.starts_at.is_none_or(|at| at <= now)
        && row.ends_at.is_none_or(|at| at > now)
}

fn public_not_found(slug: &str) -> Error {
    Error::not_found(
        "catalog.collection_not_found",
        "There is no product collection `{id}` on this website.",
    )
    .with_param("id", slug)
}

impl CatalogService {
    /// Bộ sưu tập công khai có slug `slug` ở ngôn ngữ `locale` trên website `site` lúc `now`.
    pub(crate) async fn find_public_collection(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        slug: &str,
        now: DateTime<Utc>,
    ) -> Result<(collection::Model, collection_text::Model), Error> {
        let locale = self.locale(locale)?;
        let read = tenant.database().read();
        let text = collection_text::Entity::find()
            .filter(collection_text::Column::Locale.eq(locale.as_str()))
            .filter(collection_text::Column::Slug.eq(slug))
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(|| public_not_found(slug))?;
        let row = collection::Entity::find_by_id(text.collection_id)
            .scoped(tenant)
            .one(read)
            .await?
            .filter(|row| shown(row, now))
            // Bộ sưu tập của một nhánh chỉ hiện trên website của nhánh đó và bên dưới.
            .filter(|row| site.lineage_ids().contains(&row.org_id))
            .ok_or_else(|| public_not_found(slug))?;
        Ok((row, text))
    }

    /// Bộ sưu tập công khai kèm trang sản phẩm `page` (thứ tự `order` của `page` do người gọi đặt
    /// theo `sort` của bộ sưu tập — xem [`ascending`]).
    pub(crate) async fn public_collection(
        &self,
        tenant: &TenantContext,
        site: &Site,
        found: (collection::Model, collection_text::Model),
        page: &PageRequest,
        now: DateTime<Utc>,
    ) -> Result<PublicCollection, Error> {
        let (row, text) = found;
        let products =
            self.collection_products(tenant, site, &text.locale, &row, page, now).await?;
        Ok(PublicCollection { collection: row, text, products })
    }

    async fn collection_products(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        row: &collection::Model,
        page: &PageRequest,
        now: DateTime<Utc>,
    ) -> Result<Page<PublicCard>, Error> {
        let mut filter = if row.kind == "rule" {
            let rules: Rules = serde_json::from_str(&row.rules).unwrap_or_default();
            rules.filter(locale, now)
        } else {
            PublicFilter {
                locale: locale.to_owned(),
                collection: Some(row.id),
                ..Default::default()
            }
        };
        filter.sort = match row.sort.as_str() {
            "price_asc" => PublicSort::PriceAsc,
            "price_desc" => PublicSort::PriceDesc,
            "best_selling" => PublicSort::BestSelling,
            _ => PublicSort::Newest,
        };
        if row.sort != "manual" {
            return self.public_products(tenant, site, &filter, page).await;
        }
        // Thứ tự chọn tay: trang trên bảng mục, chỉ sản phẩm còn hiện trên website.
        let condition = self.filter_condition(tenant, site, &filter, Skip::default()).await?;
        let visible = Query::select()
            .column(storefront_entry::Column::ProductId)
            .from(storefront_entry::Entity)
            .and_where(storefront_entry::Column::TenantId.eq(tenant.id().as_uuid()))
            .cond_where(condition.clone())
            .to_owned();
        let items = collection_item::Entity::find()
            .filter(collection_item::Column::CollectionId.eq(row.id))
            .filter(collection_item::Column::ProductId.in_subquery(visible))
            .scoped(tenant);
        let read = tenant.database().read();
        let Page { data, pagination } = fetch_page_by(
            items,
            collection_item::Column::Position,
            |item: &collection_item::Model| SortValue::Integer(i64::from(item.position)),
            collection_item::Column::ProductId,
            |item: &collection_item::Model| item.product_id,
            page,
            read,
        )
        .await?;
        let ids: Vec<Uuid> = data.iter().map(|item| item.product_id).collect();
        let mut entries = storefront_entry::Entity::find()
            .filter(condition)
            .filter(storefront_entry::Column::ProductId.is_in(ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?;
        entries.sort_by_key(|entry| ids.iter().position(|id| *id == entry.product_id));
        let data = self.cards(tenant, locale, entries).await?;
        Ok(Page { data, pagination })
    }
}
