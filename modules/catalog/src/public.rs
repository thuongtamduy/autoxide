//! Đọc công khai cho website (không cần đăng nhập): chỉ sản phẩm `active`, thấy được ở website
//! của request, có bản dịch bằng ngôn ngữ hỏi. Giá, ẩn / hiện và cột sắp xếp đọc ở bảng dẫn xuất
//! `catalog_storefront_entries` (xem `storefront.rs`); mọi bộ lọc thành subquery có index, không
//! đổ danh sách ID ra bộ nhớ.

use std::collections::{BTreeMap, HashMap};

use autoxide_kernel::error::Error;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, Site, TenantContext};
use chrono::Utc;
use sea_orm::sea_query::Query;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::Value;
use uuid::Uuid;

use crate::entity::{
    category, category_path, category_translation, price_list, product, product_attribute,
    product_media, store_override, storefront_entry, translation, variant,
};
use crate::errors;
use crate::public_filter::Skip;
use crate::service::CatalogService;

/// Độ dài tối đa của câu tìm (một câu `LIKE` dài là một lần quét đắt, ai gọi cũng được).
pub(crate) const QUERY_MAX: usize = 200;
/// Số bộ lọc thuộc tính tối đa trong một request.
pub(crate) const MAX_ATTRIBUTE_FILTERS: usize = 10;

/// Cách sắp danh sách công khai.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PublicSort {
    #[default]
    Newest,
    PriceAsc,
    PriceDesc,
    BestSelling,
    Rating,
}

/// Sản phẩm của chính website hay của người bán bên dưới nó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SoldBy {
    /// Của nút website hay nút cấp trên.
    Own,
    /// Của người bán (nút) bên dưới website.
    Marketplace,
}

/// Bộ lọc của `GET /public/products`.
#[derive(Debug, Clone, Default)]
pub(crate) struct PublicFilter {
    pub locale: String,
    pub category: Option<String>,
    /// (mã thuộc tính, các giá trị — khớp một trong số đó).
    pub attributes: Vec<(String, Vec<String>)>,
    pub price_min: Option<i64>,
    pub price_max: Option<i64>,
    pub q: Option<String>,
    pub sort: PublicSort,
    pub on_sale: bool,
    pub seller: Option<Uuid>,
    pub sold_by: Option<SoldBy>,
    /// Điểm đánh giá tối thiểu × 100.
    pub min_rating: Option<i32>,
    /// Bộ sưu tập (bước 55): thuộc một trong các danh mục này (gồm danh mục con).
    pub category_ids: Vec<Uuid>,
    /// Bộ sưu tập: lên kệ từ thời điểm này.
    pub published_after: Option<chrono::DateTime<Utc>>,
    /// Bộ sưu tập chọn tay: chỉ sản phẩm đã chọn của bộ sưu tập này.
    pub collection: Option<Uuid>,
}

/// Một sản phẩm trên danh sách công khai.
#[derive(Debug, Clone)]
pub(crate) struct PublicCard {
    pub entry: storefront_entry::Model,
    pub product: product::Model,
    pub text: translation::Model,
    pub image: Option<product_media::Model>,
}

/// Một sản phẩm đầy đủ cho trang sản phẩm.
#[derive(Debug, Clone)]
pub(crate) struct PublicProduct {
    pub card: PublicCard,
    /// Bản dịch khác (ngôn ngữ → slug), cho thẻ `hreflang`.
    pub translations: Vec<translation::Model>,
    pub variants: Vec<variant::Model>,
    pub options: Vec<crate::variants::OptionDetail>,
    /// Giá hiện tại theo biến thể (không nhóm khách); biến thể không có giá thì không có mục.
    pub prices: HashMap<Uuid, crate::api::PriceSnapshot>,
    pub media: Vec<product_media::Model>,
    pub categories: Vec<(category::Model, category_translation::Model)>,
    pub attributes: Vec<PublicAttribute>,
    /// Bản ghi đè `alt` của ảnh, mọi ngôn ngữ (trang lọc theo ngôn ngữ của nó).
    pub media_alts: Vec<crate::entity::media_alt::Model>,
}

/// Một thuộc tính hiển thị trên trang sản phẩm.
#[derive(Debug, Clone)]
pub(crate) struct PublicAttribute {
    pub code: String,
    pub name: String,
    pub unit: Option<String>,
    /// Giá trị dạng như khi ghi: chữ, số, số thập phân dạng chữ, `true`/`false`, ngày, mã lựa chọn.
    pub value: Value,
    /// Nhãn hiển thị bằng ngôn ngữ hỏi.
    pub label: String,
}

/// Một danh mục công khai.
#[derive(Debug, Clone)]
pub(crate) struct PublicCategory {
    pub category: category::Model,
    pub text: category_translation::Model,
}

/// Một dòng của sitemap.
#[derive(Debug, Clone)]
pub(crate) struct SitemapItem {
    pub entry: storefront_entry::Model,
    pub translations: Vec<translation::Model>,
    pub modified_at: chrono::DateTime<Utc>,
}

/// Tên theo ngôn ngữ từ JSON `{locale: tên}`; thiếu thì tên đầu tiên.
pub(crate) fn pick(names: &str, locale: &str) -> String {
    let map: BTreeMap<String, String> = serde_json::from_str(names).unwrap_or_default();
    map.get(locale).or_else(|| map.values().next()).cloned().unwrap_or_default()
}

impl CatalogService {
    /// Nút bán của website `site`: nút gần nhất trong chuỗi có bảng giá đang bật hay tuỳ biến ẩn /
    /// hiện; không có thì gốc.
    pub(crate) async fn storefront_of(
        &self,
        tenant: &TenantContext,
        site: &Site,
    ) -> Result<OrgId, Error> {
        let read = tenant.database().read();
        let lineage = site.lineage_ids();
        let mut found: Vec<Uuid> = price_list::Entity::find()
            .select_only()
            .column(price_list::Column::OrgId)
            .filter(price_list::Column::Active.eq(true))
            .filter(price_list::Column::OrgId.is_in(lineage.clone()))
            .scoped(tenant)
            .distinct()
            .into_tuple()
            .all(read)
            .await?;
        let overrides: Vec<Uuid> = store_override::Entity::find()
            .select_only()
            .column(store_override::Column::OrgId)
            .filter(store_override::Column::OrgId.is_in(lineage))
            .scoped(tenant)
            .distinct()
            .into_tuple()
            .all(read)
            .await?;
        found.extend(overrides);
        Ok(site
            .lineage()
            .iter()
            .find(|org| found.contains(&org.as_uuid()))
            .copied()
            .unwrap_or_else(|| *site.lineage().last().unwrap_or(&site.org())))
    }

    /// Điều kiện "dòng storefront thấy được ở website `site`": của nút bán của nó, đang hiện,
    /// sản phẩm của chuỗi tổ tiên hay của nhánh bên dưới website.
    pub(crate) async fn visible_on(
        &self,
        tenant: &TenantContext,
        site: &Site,
    ) -> Result<Condition, Error> {
        let pricing = self.storefront_of(tenant, site).await?;
        Ok(Condition::all()
            .add(storefront_entry::Column::OrgId.eq(pricing.as_uuid()))
            .add(storefront_entry::Column::Visible.eq(true))
            .add(
                Condition::any()
                    .add(storefront_entry::Column::OwnerOrgId.is_in(site.lineage_ids()))
                    .add(OrgTree::subtree_condition(
                        tenant.id(),
                        &[site.org()],
                        storefront_entry::Column::OwnerOrgId,
                    )),
            ))
    }

    /// Danh mục hiện được theo slug và ngôn ngữ.
    pub(crate) async fn public_category(
        &self,
        tenant: &TenantContext,
        locale: &str,
        slug: &str,
    ) -> Result<PublicCategory, Error> {
        let read = tenant.database().read();
        let not_found = || errors::category_slug_not_found(slug);
        let text = category_translation::Entity::find()
            .filter(category_translation::Column::Locale.eq(locale))
            .filter(category_translation::Column::Slug.eq(slug))
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(not_found)?;
        let category = category::Entity::find_by_id(text.category_id)
            .scoped(tenant)
            .one(read)
            .await?
            .filter(|row| row.visible)
            .ok_or_else(not_found)?;
        // Danh mục cha bị ẩn thì cả nhánh ẩn.
        let hidden_above = category_path::Entity::find()
            .filter(category_path::Column::DescendantId.eq(category.id))
            .filter(
                category_path::Column::AncestorId.in_subquery(
                    Query::select()
                        .column(category::Column::Id)
                        .from(category::Entity)
                        .and_where(category::Column::TenantId.eq(tenant.id().as_uuid()))
                        .and_where(category::Column::Visible.eq(false))
                        .to_owned(),
                ),
            )
            .scoped(tenant)
            .one(read)
            .await?;
        if hidden_above.is_some() {
            return Err(not_found());
        }
        Ok(PublicCategory { category, text })
    }

    /// Cây danh mục hiện được, bằng ngôn ngữ `locale` (danh mục chưa có tên bằng ngôn ngữ đó, hay
    /// nằm dưới danh mục ẩn, thì bỏ). Sắp theo độ sâu rồi thứ tự.
    pub(crate) async fn public_categories(
        &self,
        tenant: &TenantContext,
        locale: &str,
    ) -> Result<Vec<PublicCategory>, Error> {
        let read = tenant.database().read();
        let rows = category::Entity::find()
            .scoped(tenant)
            .order_by_asc(category::Column::Depth)
            .order_by_asc(category::Column::Position)
            .order_by_asc(category::Column::Id)
            .all(read)
            .await?;
        let mut texts: HashMap<Uuid, category_translation::Model> =
            category_translation::Entity::find()
                .filter(category_translation::Column::Locale.eq(locale))
                .scoped(tenant)
                .all(read)
                .await?
                .into_iter()
                .map(|text| (text.category_id, text))
                .collect();
        // Sắp theo độ sâu: cha luôn được xét trước con.
        let mut shown: Vec<Uuid> = Vec::new();
        let mut out = Vec::new();
        for category in rows {
            let parent_shown = category.parent_id.is_none_or(|parent| shown.contains(&parent));
            if !category.visible || !parent_shown {
                continue;
            }
            let Some(text) = texts.remove(&category.id) else { continue };
            shown.push(category.id);
            out.push(PublicCategory { category, text });
        }
        Ok(out)
    }

    /// Một trang sản phẩm công khai của website `site`.
    pub(crate) async fn public_products(
        &self,
        tenant: &TenantContext,
        site: &Site,
        filter: &PublicFilter,
        page: &PageRequest,
    ) -> Result<Page<PublicCard>, Error> {
        let condition = self.filter_condition(tenant, site, filter, Skip::default()).await?;
        let select = storefront_entry::Entity::find().filter(condition).scoped(tenant);
        let read = tenant.database().read();
        let id = storefront_entry::Column::ProductId;
        let id_of = |row: &storefront_entry::Model| row.product_id;
        let entries = match filter.sort {
            PublicSort::Newest => {
                let key = |row: &storefront_entry::Model| SortValue::Time(row.published_at);
                fetch_page_by(
                    select,
                    storefront_entry::Column::PublishedAt,
                    key,
                    id,
                    id_of,
                    page,
                    read,
                )
                .await?
            }
            PublicSort::PriceAsc | PublicSort::PriceDesc => {
                let key = |row: &storefront_entry::Model| SortValue::Integer(row.min_price_minor);
                fetch_page_by(
                    select,
                    storefront_entry::Column::MinPriceMinor,
                    key,
                    id,
                    id_of,
                    page,
                    read,
                )
                .await?
            }
            PublicSort::BestSelling => {
                let key = |row: &storefront_entry::Model| SortValue::Integer(row.best_selling);
                fetch_page_by(
                    select,
                    storefront_entry::Column::BestSelling,
                    key,
                    id,
                    id_of,
                    page,
                    read,
                )
                .await?
            }
            PublicSort::Rating => {
                let key =
                    |row: &storefront_entry::Model| SortValue::Integer(i64::from(row.rating_x100));
                fetch_page_by(
                    select,
                    storefront_entry::Column::RatingX100,
                    key,
                    id,
                    id_of,
                    page,
                    read,
                )
                .await?
            }
        };
        let Page { data, pagination } = entries;
        let data = self.cards(tenant, &filter.locale, data).await?;
        Ok(Page { data, pagination })
    }

    /// Ghép sản phẩm, bản dịch và ảnh đầu cho các dòng storefront (giữ thứ tự).
    pub(crate) async fn cards(
        &self,
        tenant: &TenantContext,
        locale: &str,
        entries: Vec<storefront_entry::Model>,
    ) -> Result<Vec<PublicCard>, Error> {
        let read = tenant.database().read();
        let ids: Vec<Uuid> = entries.iter().map(|row| row.product_id).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut products: HashMap<Uuid, product::Model> = product::Entity::find()
            .filter(product::Column::Id.is_in(ids.clone()))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        let mut texts: HashMap<Uuid, translation::Model> = translation::Entity::find()
            .filter(translation::Column::ProductId.is_in(ids.clone()))
            .filter(translation::Column::Locale.eq(locale))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.product_id, row))
            .collect();
        let mut images: HashMap<Uuid, product_media::Model> = HashMap::new();
        let media = product_media::Entity::find()
            .filter(product_media::Column::ProductId.is_in(ids))
            .scoped(tenant)
            .order_by_desc(product_media::Column::Position)
            .all(read)
            .await?;
        // Thứ tự giảm: ảnh vị trí nhỏ nhất ghi sau cùng, thành ảnh đại diện.
        for row in media {
            images.insert(row.product_id, row);
        }
        Ok(entries
            .into_iter()
            .filter_map(|entry| {
                let product = products.remove(&entry.product_id)?;
                let text = texts.remove(&entry.product_id)?;
                let image = images.remove(&entry.product_id);
                Some(PublicCard { entry, product, text, image })
            })
            .collect())
    }

    /// Sản phẩm công khai theo slug ở ngôn ngữ `locale`, trên website `site`.
    pub(crate) async fn public_product(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: &str,
        slug: &str,
    ) -> Result<PublicProduct, Error> {
        let read = tenant.database().read();
        let not_found = || errors::product_slug_not_found(slug);
        let text = translation::Entity::find()
            .filter(translation::Column::Locale.eq(locale))
            .filter(translation::Column::Slug.eq(slug))
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(not_found)?;
        let entry = storefront_entry::Entity::find()
            .filter(self.visible_on(tenant, site).await?)
            .filter(storefront_entry::Column::ProductId.eq(text.product_id))
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(not_found)?;
        let mut cards = self.cards(tenant, locale, vec![entry]).await?;
        let card = cards.pop().ok_or_else(not_found)?;
        let detail = self.detail(tenant, card.product.clone()).await?;
        let variants: Vec<variant::Model> =
            detail.variants.into_iter().filter(|row| row.active).collect();
        let currency = autoxide_kernel::money::Currency::parse(&card.entry.currency)
            .map_err(|_| Error::internal("storefront currency is not a currency code"))?;
        let query = crate::api::PriceQuery::new(
            OrgId::from_uuid(card.entry.org_id),
            currency,
            Utc::now(),
            variants.iter().map(|row| row.id).collect(),
        );
        let prices = self
            .resolve_prices(tenant, &query, false)
            .await?
            .into_iter()
            .map(|price| (price.variant_id, price))
            .collect();
        let category_rows = category::Entity::find()
            .filter(category::Column::Id.is_in(detail.category_ids.clone()))
            .filter(category::Column::Visible.eq(true))
            .scoped(tenant)
            .all(read)
            .await?;
        let category_texts = category_translation::Entity::find()
            .filter(category_translation::Column::CategoryId.is_in(detail.category_ids.clone()))
            .filter(category_translation::Column::Locale.eq(locale))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut categories = Vec::new();
        for id in &detail.category_ids {
            let row = category_rows.iter().find(|row| row.id == *id);
            let text = category_texts.iter().find(|text| text.category_id == *id);
            if let (Some(row), Some(text)) = (row, text) {
                categories.push((row.clone(), text.clone()));
            }
        }
        let attributes = self.public_attributes(tenant, card.product.id, locale).await?;
        Ok(PublicProduct {
            translations: detail.translations,
            variants,
            options: detail.options,
            prices,
            media: detail.media,
            categories,
            attributes,
            media_alts: detail.media_alts,
            card,
        })
    }

    /// Giá trị thuộc tính của sản phẩm, kèm tên và nhãn bằng ngôn ngữ `locale`.
    async fn public_attributes(
        &self,
        tenant: &TenantContext,
        product_id: Uuid,
        locale: &str,
    ) -> Result<Vec<PublicAttribute>, Error> {
        let read = tenant.database().read();
        let rows = self.product_attributes(read, tenant, product_id).await?;
        let ids: Vec<Uuid> = rows.iter().map(|row| row.attribute_id).collect();
        let details = self.attribute_details(read, tenant, &ids).await?;
        Ok(present_attributes(rows, &details, locale))
    }

    /// Một trang sitemap: mọi sản phẩm thấy được ở website, kèm mọi bản dịch (hay một ngôn ngữ).
    pub(crate) async fn public_sitemap(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: Option<&str>,
        page: &PageRequest,
    ) -> Result<Page<SitemapItem>, Error> {
        let mut condition = self.visible_on(tenant, site).await?;
        if let Some(locale) = locale {
            let texts = Query::select()
                .column(translation::Column::ProductId)
                .from(translation::Entity)
                .and_where(translation::Column::TenantId.eq(tenant.id().as_uuid()))
                .and_where(translation::Column::Locale.eq(locale))
                .to_owned();
            condition = condition.add(storefront_entry::Column::ProductId.in_subquery(texts));
        }
        let select = storefront_entry::Entity::find().filter(condition).scoped(tenant);
        let read = tenant.database().read();
        let key = |row: &storefront_entry::Model| SortValue::Time(row.published_at);
        let Page { data, pagination } = fetch_page_by(
            select,
            storefront_entry::Column::PublishedAt,
            key,
            storefront_entry::Column::ProductId,
            |row| row.product_id,
            page,
            read,
        )
        .await?;
        let ids: Vec<Uuid> = data.iter().map(|row| row.product_id).collect();
        let mut select = translation::Entity::find()
            .filter(translation::Column::ProductId.is_in(ids.clone()))
            .scoped(tenant);
        if let Some(locale) = locale {
            select = select.filter(translation::Column::Locale.eq(locale));
        }
        let texts = select.order_by_asc(translation::Column::Locale).all(read).await?;
        let products: HashMap<Uuid, chrono::DateTime<Utc>> = product::Entity::find()
            .filter(product::Column::Id.is_in(ids))
            .scoped(tenant)
            .all(read)
            .await?
            .into_iter()
            .map(|row| (row.id, row.updated_at))
            .collect();
        let data = data
            .into_iter()
            .map(|entry| {
                let translations: Vec<translation::Model> = texts
                    .iter()
                    .filter(|text| text.product_id == entry.product_id)
                    .cloned()
                    .collect();
                let text_at = translations.iter().map(|text| text.updated_at).max();
                let product_at = products.get(&entry.product_id).copied();
                let modified_at = [Some(entry.refreshed_at), text_at, product_at]
                    .into_iter()
                    .flatten()
                    .max()
                    .unwrap_or(entry.refreshed_at);
                SitemapItem { entry, translations, modified_at }
            })
            .collect();
        Ok(Page { data, pagination })
    }
}

impl CatalogService {
    /// Ảnh công khai theo ID (bỏ ảnh không còn hay không còn công khai). Chưa cài `media`: rỗng.
    pub(crate) async fn public_media(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, autoxide_media::api::MediaRef>, Error> {
        let Some(media) = &self.media else { return Ok(HashMap::new()) };
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        Ok(media
            .find_many(tenant, ids)
            .await?
            .into_iter()
            .filter(|item| item.visibility == autoxide_media::api::MediaVisibility::Public)
            .map(|item| (item.id, item))
            .collect())
    }

    /// Như [`public_media`](Self::public_media), với `alt` của media bằng ngôn ngữ `locale`.
    pub(crate) async fn public_media_in(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<HashMap<Uuid, autoxide_media::api::MediaRef>, Error> {
        let Some(media) = &self.media else { return Ok(HashMap::new()) };
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        Ok(media
            .find_many_in(tenant, ids, locale)
            .await?
            .into_iter()
            .filter(|item| item.visibility == autoxide_media::api::MediaVisibility::Public)
            .map(|item| (item.id, item))
            .collect())
    }

    /// Tên của các nút bán (người bán). Chưa cài `organizations`: rỗng.
    pub(crate) async fn seller_names(
        &self,
        tenant: &TenantContext,
        owners: &[Uuid],
    ) -> Result<HashMap<Uuid, String>, Error> {
        let Some(directory) = &self.organizations else { return Ok(HashMap::new()) };
        // Một lần tra cho cả trang (D-20261120-step39-hot-paths; trước: một câu mỗi người bán).
        let mut wanted: Vec<OrgId> = owners.iter().map(|owner| OrgId::from_uuid(*owner)).collect();
        wanted.sort_unstable_by_key(|id| id.as_uuid());
        wanted.dedup();
        let mut names = HashMap::new();
        for unit in directory.units(tenant, &wanted).await? {
            names.insert(unit.id.as_uuid(), unit.name);
        }
        Ok(names)
    }
}

/// 1250 với `scale` 2 → `"12.5"` (không qua số thực).
fn unscale(number: i64, scale: i32) -> String {
    let Ok(scale) = u32::try_from(scale) else { return number.to_string() };
    if scale == 0 {
        return number.to_string();
    }
    let factor = 10_i64.pow(scale);
    let sign = if number < 0 { "-" } else { "" };
    let whole = (number / factor).abs();
    let fraction = (number % factor).abs();
    let fraction = format!("{fraction:0width$}", width = scale as usize);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() { format!("{sign}{whole}") } else { format!("{sign}{whole}.{fraction}") }
}

/// Thuộc tính hiển thị từ các dòng giá trị (của sản phẩm hay của biến thể), ở ngôn ngữ `locale`.
pub(crate) fn present_attributes(
    rows: Vec<product_attribute::Model>,
    details: &HashMap<Uuid, crate::attributes::AttributeDetail>,
    locale: &str,
) -> Vec<PublicAttribute> {
    let mut out: Vec<PublicAttribute> = Vec::new();
    for row in rows {
        let Some(detail) = details.get(&row.attribute_id) else { continue };
        let attribute = &detail.attribute;
        let option =
            row.option_id.and_then(|id| detail.options.iter().find(|option| option.id == id));
        let (value, label) = if let Some(option) = option {
            (Value::String(option.code.clone()), pick(&option.labels, locale))
        } else if let Some(text) = &row.value_text {
            (Value::String(text.clone()), text.clone())
        } else if let Some(number) = row.value_integer {
            let shown = if attribute.data_type == "decimal" {
                unscale(number, attribute.scale)
            } else {
                number.to_string()
            };
            let value = if attribute.data_type == "decimal" {
                Value::String(shown.clone())
            } else {
                Value::from(number)
            };
            (value, shown)
        } else if let Some(flag) = row.value_boolean {
            (Value::Bool(flag), flag.to_string())
        } else if let Some(date) = row.value_date {
            (Value::String(date.to_string()), date.to_string())
        } else {
            continue;
        };
        // `options`: nhiều dòng cùng thuộc tính gộp thành một mục, giá trị là danh sách.
        if attribute.data_type == "options" {
            if let Some(existing) = out.iter_mut().find(|item| item.code == attribute.code) {
                if let Value::Array(items) = &mut existing.value {
                    items.push(value);
                }
                existing.label = format!("{}, {label}", existing.label);
                continue;
            }
            out.push(PublicAttribute {
                code: attribute.code.clone(),
                name: pick(&attribute.names, locale),
                unit: attribute.unit.clone(),
                value: Value::Array(vec![value]),
                label,
            });
            continue;
        }
        out.push(PublicAttribute {
            code: attribute.code.clone(),
            name: pick(&attribute.names, locale),
            unit: attribute.unit.clone(),
            value,
            label,
        });
    }
    out
}

#[cfg(test)]
mod unit {
    use super::unscale;

    #[test]
    fn doi_so_thap_phan_khong_qua_so_thuc() {
        assert_eq!(unscale(1250, 2), "12.5");
        assert_eq!(unscale(1200, 2), "12");
        assert_eq!(unscale(-5, 2), "-0.05");
        assert_eq!(unscale(7, 0), "7");
    }
}
