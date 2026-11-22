//! Khối bán hàng của `catalog` cho trang dựng từ khối (bước 67, bước 41): lưới sản phẩm, thẻ một
//! sản phẩm, bộ sưu tập. Dữ liệu đọc như đọc công khai (`GET /public/products`): chỉ sản phẩm đang
//! hiện ở website, có bản dịch bằng ngôn ngữ của trang; giá luôn là giá hiện tại — trang không bao
//! giờ viết cứng giá. Mỗi loại khối một lần gọi cho cả trang; thẻ sản phẩm và lưới chọn tay gom
//! mọi ID của trang vào một lần đọc.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::blocks::{BlockCategory, BlockData, BlockRef, BlockRequest, BlockType};
use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Order, PageRequest};
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{collection, collection_text, storefront_entry};
use crate::http::public::{PublicCardResponse, cards};
use crate::public::{PublicFilter, PublicSort};
use crate::public_filter::Skip;
use crate::service::CatalogService;

/// Số sản phẩm tối đa của một khối.
const MAX_PRODUCTS: u32 = 24;
/// Số sản phẩm mặc định của lưới và bộ sưu tập.
const DEFAULT_PRODUCTS: u32 = 8;

pub(crate) const PRODUCT_GRID: BlockType = BlockType::new(
    "catalog.product_grid",
    BlockCategory::Commerce,
    "Product grid",
    "Use to show several products: hand-picked ones, a category, products on sale or best sellers; prices always come from the catalog.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "title": {"type": "string", "maxLength": 200},
            "product_ids": {"type": "array", "maxItems": 24, "items": {"type": "string", "format": "uuid"}, "description": "Hand-picked products, in order; when set, the other filters are ignored."},
            "category_id": {"type": "string", "format": "uuid", "description": "Only products of this category (and its subcategories)."},
            "on_sale": {"type": "boolean", "description": "Only products with an automatic discount."},
            "sort": {"enum": ["newest", "price_asc", "price_desc", "best_selling", "rating"]},
            "limit": {"type": "integer", "minimum": 1, "maximum": 24},
            "columns": {"type": "integer", "minimum": 1, "maximum": 6, "x-responsive": true}
        }
    }"#,
)
.with_data();

pub(crate) const PRODUCT_CARD: BlockType = BlockType::new(
    "catalog.product_card",
    BlockCategory::Commerce,
    "Product",
    "Use to feature one product with its picture, current price and rating.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["product_id"],
        "properties": {
            "product_id": {"type": "string", "format": "uuid"},
            "show_description": {"type": "boolean"}
        }
    }"#,
)
.with_data();

pub(crate) const COLLECTION: BlockType = BlockType::new(
    "catalog.collection",
    BlockCategory::Commerce,
    "Collection",
    "Use to show the products of a collection the shop prepared, in the collection's order.",
    r#"{
        "type": "object",
        "additionalProperties": false,
        "required": ["collection_id"],
        "properties": {
            "collection_id": {"type": "string", "format": "uuid"},
            "limit": {"type": "integer", "minimum": 1, "maximum": 24},
            "columns": {"type": "integer", "minimum": 1, "maximum": 6, "x-responsive": true}
        }
    }"#,
)
.with_data();

pub(crate) fn all() -> Vec<BlockType> {
    vec![PRODUCT_GRID, PRODUCT_CARD, COLLECTION]
}

/// Bộ trả dữ liệu cho cả ba khối.
pub(crate) struct CatalogBlocks(pub(crate) Arc<CatalogService>);

fn limit_of(props: &Value) -> u32 {
    props
        .get("limit")
        .and_then(Value::as_u64)
        .and_then(|limit| u32::try_from(limit).ok())
        .unwrap_or(DEFAULT_PRODUCTS)
        .clamp(1, MAX_PRODUCTS)
}

fn uuid_of(value: Option<&Value>) -> Option<Uuid> {
    value.and_then(Value::as_str).and_then(|text| Uuid::parse_str(text).ok())
}

fn to_value(card: &PublicCardResponse) -> Value {
    serde_json::to_value(card).unwrap_or(Value::Null)
}

impl CatalogBlocks {
    /// Thẻ của các sản phẩm `ids` còn hiện ở website, theo ID (một lần đọc cho mọi ID).
    async fn cards_by_id(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        ids: Vec<Uuid>,
    ) -> Result<HashMap<Uuid, Value>, Error> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let service = &self.0;
        let filter = PublicFilter { locale: locale.to_owned(), ..PublicFilter::default() };
        let condition = service
            .filter_condition(tenant, site, &filter, Skip::default())
            .await?
            .add(storefront_entry::Column::ProductId.is_in(ids));
        let entries = storefront_entry::Entity::find()
            .filter(condition)
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let rows = service.cards(tenant, locale, entries).await?;
        let shown = cards(service, tenant, site, locale, &rows).await?;
        Ok(shown.iter().map(|card| (card.id, to_value(card))).collect())
    }

    async fn product_cards(
        &self,
        request: &BlockRequest<'_>,
        locale: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let ids: Vec<Uuid> =
            items.iter().filter_map(|item| uuid_of(item.props.get("product_id"))).collect();
        let found = self.cards_by_id(request.tenant, request.site, locale, ids).await?;
        Ok(items
            .iter()
            .map(|item| {
                let card = uuid_of(item.props.get("product_id")).and_then(|id| found.get(&id))?;
                Some(json!({"product": card}))
            })
            .collect())
    }

    async fn grids(
        &self,
        request: &BlockRequest<'_>,
        locale: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let picked = |item: &BlockRef| -> Vec<Uuid> {
            item.props
                .get("product_ids")
                .and_then(Value::as_array)
                .map(|ids| ids.iter().filter_map(|id| uuid_of(Some(id))).collect())
                .unwrap_or_default()
        };
        // Mọi sản phẩm chọn tay của mọi lưới trong trang: một lần đọc.
        let all_picked: Vec<Uuid> = items.iter().flat_map(&picked).collect();
        let found = self.cards_by_id(request.tenant, request.site, locale, all_picked).await?;
        let mut answers = Vec::with_capacity(items.len());
        for item in items {
            let ids = picked(item);
            let products: Vec<Value> = if ids.is_empty() {
                self.filtered(request, locale, &item.props).await?
            } else {
                ids.iter()
                    .filter_map(|id| found.get(id).cloned())
                    .take(limit_of(&item.props) as usize)
                    .collect()
            };
            answers.push(Some(json!({"products": products})));
        }
        Ok(answers)
    }

    /// Lưới theo bộ lọc: một trang của danh sách công khai.
    async fn filtered(
        &self,
        request: &BlockRequest<'_>,
        locale: &str,
        props: &Value,
    ) -> Result<Vec<Value>, Error> {
        let sort = match props.get("sort").and_then(Value::as_str) {
            Some("price_asc") => PublicSort::PriceAsc,
            Some("price_desc") => PublicSort::PriceDesc,
            Some("best_selling") => PublicSort::BestSelling,
            Some("rating") => PublicSort::Rating,
            _ => PublicSort::Newest,
        };
        let filter = PublicFilter {
            locale: locale.to_owned(),
            sort,
            on_sale: props.get("on_sale") == Some(&Value::Bool(true)),
            category_ids: uuid_of(props.get("category_id")).into_iter().collect(),
            ..PublicFilter::default()
        };
        let order = if sort == PublicSort::PriceAsc { Order::Asc } else { Order::Desc };
        let page = PageRequest::first(limit_of(props), order);
        let service = &self.0;
        let found = service.public_products(request.tenant, request.site, &filter, &page).await?;
        let shown = cards(service, request.tenant, request.site, locale, &found.data).await?;
        Ok(shown.iter().map(to_value).collect())
    }

    async fn collections(
        &self,
        request: &BlockRequest<'_>,
        locale: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        let tenant = request.tenant;
        let read = tenant.database().read();
        let ids: Vec<Uuid> =
            items.iter().filter_map(|item| uuid_of(item.props.get("collection_id"))).collect();
        let now = Timestamp::now();
        let rows: HashMap<Uuid, collection::Model> = collection::Entity::find()
            .filter(collection::Column::Id.is_in(ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .filter(|row| crate::collections::public::shown(row, now))
            .filter(|row| request.site.lineage_ids().contains(&row.org_id))
            .map(|row| (row.id, row))
            .collect();
        let mut texts: HashMap<Uuid, collection_text::Model> = collection_text::Entity::find()
            .filter(collection_text::Column::CollectionId.is_in(ids))
            .filter(collection_text::Column::Locale.eq(locale))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|text| (text.collection_id, text))
            .collect();
        let service = &self.0;
        let mut answers = Vec::with_capacity(items.len());
        for item in items {
            let id = uuid_of(item.props.get("collection_id"));
            let (Some(row), Some(text)) =
                (id.and_then(|id| rows.get(&id)), id.and_then(|id| texts.remove(&id)))
            else {
                answers.push(None);
                continue;
            };
            let ascending = crate::collections::public::ascending(&row.sort);
            let page = PageRequest::first(
                limit_of(&item.props),
                if ascending { Order::Asc } else { Order::Desc },
            );
            let found = service
                .public_collection(tenant, request.site, (row.clone(), text), &page, now)
                .await?;
            let products =
                cards(service, tenant, request.site, locale, &found.products.data).await?;
            answers.push(Some(json!({
                "collection": {
                    "id": found.collection.id,
                    "title": found.text.title,
                    "slug": found.text.slug,
                    "description": found.text.description,
                    "media_id": found.collection.media_id,
                },
                "products": products.iter().map(to_value).collect::<Vec<_>>(),
            })));
        }
        Ok(answers)
    }
}

#[async_trait::async_trait]
impl BlockData for CatalogBlocks {
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error> {
        // Ngôn ngữ trang không phải ngôn ngữ của catalog: không có sản phẩm nào để hiện.
        let Ok(locale) = self.0.locale(request.locale) else {
            return Ok(vec![None; items.len()]);
        };
        match block {
            "catalog.product_card" => self.product_cards(request, &locale, items).await,
            "catalog.product_grid" => self.grids(request, &locale, items).await,
            "catalog.collection" => self.collections(request, &locale, items).await,
            _ => Ok(vec![None; items.len()]),
        }
    }
}
