//! Endpoint công khai cho website (không cần đăng nhập; gửi token thì token phải đúng), dưới
//! `/api/v1/catalog/public`. Chỉ sản phẩm đang bán, thấy được ở website của request (`Site`: tên
//! miền hay `?site=`). Có `ETag` là dấu vân tay của cả câu trả lời (→ 304) và
//! `Cache-Control: public, no-cache` như `content` (D-102).

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Data, Problem};
use autoxide_kernel::tenancy::{Site, SiteQuery, TenantContext};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::entity::{product_media, translation};
use crate::public::{
    MAX_ATTRIBUTE_FILTERS, PublicCard, PublicFilter, PublicProduct, PublicSort, QUERY_MAX, SoldBy,
    pick,
};
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_public_products))
        .routes(routes!(get_public_product))
        .routes(routes!(list_public_categories))
        .routes(routes!(get_public_category))
        .routes(routes!(public_sitemap))
        .with_state(service)
}

/// `Cache-Control` của mọi endpoint công khai: cache được, nhưng hỏi lại máy chủ (rẻ nhờ 304).
fn public_cache(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, no-cache"));
    response
}

/// `ETag` mạnh từ nội dung: SHA-256 của ID khách và JSON, 16 byte đầu, base64url, trong ngoặc kép
/// (như `content`). Có ID khách: hai khách trả cùng nội dung vẫn khác `ETag`. Middleware của kernel
/// trả 304 khi `If-None-Match` khớp.
fn fingerprint<T: Serialize>(tenant: &TenantContext, value: &T) -> Result<String, Error> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value)
        .map_err(|error| Error::internal(format!("cannot serialize the answer: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(tenant.id().as_uuid().as_bytes());
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let short = digest.get(..16).unwrap_or_default();
    Ok(format!("\"{}\"", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(short)))
}

/// Trả `value` kèm `ETag` và `Cache-Control` công khai.
pub(crate) fn cached<T: Serialize + IntoResponse>(
    tenant: &TenantContext,
    value: T,
) -> Result<Response, Error> {
    let etag = fingerprint(tenant, &value)?;
    let mut response = value.into_response();
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response.headers_mut().insert(header::ETAG, value);
    }
    Ok(public_cache(response))
}

/// An image, as readers load it.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicImage)]
pub(crate) struct PublicImage {
    pub media_id: Uuid,
    /// The image shows this variant.
    pub variant_id: Option<Uuid>,
    /// Where to load it (versioned: cache it for long).
    pub url: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Text alternative: the product's own, or the media library's.
    pub alt: String,
    /// Smaller copies, narrowest first, for `srcset`.
    pub sizes: Vec<PublicImageSize>,
}

/// A smaller copy of an image.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicImageSize)]
pub(crate) struct PublicImageSize {
    pub width: i32,
    pub height: i32,
    pub url: String,
    /// The same copy as WebP; `null` while it is not made yet.
    pub webp_url: Option<String>,
}

fn image(
    row: &product_media::Model,
    found: &HashMap<Uuid, autoxide_media::api::MediaRef>,
) -> Option<PublicImage> {
    let item = found.get(&row.media_id)?;
    Some(PublicImage {
        media_id: row.media_id,
        variant_id: row.variant_id,
        url: item.url.clone(),
        width: item.width,
        height: item.height,
        alt: if row.alt.is_empty() { item.alt.clone() } else { row.alt.clone() },
        sizes: item
            .variants
            .iter()
            .map(|copy| PublicImageSize {
                width: copy.width,
                height: copy.height,
                url: copy.url.clone(),
                webp_url: copy.webp_url.clone(),
            })
            .collect(),
    })
}

/// Who sells a product.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicSeller)]
pub(crate) struct PublicSeller {
    /// The organization unit selling it (filter with `seller=`).
    pub org_id: Uuid,
    /// Its name; empty when the organization module is not installed.
    pub name: String,
    /// `true` when it is the website's own unit or one above it; `false` for a seller below the
    /// website (a marketplace seller).
    pub own: bool,
}

/// A product on a public list.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicProductCard)]
pub(crate) struct PublicCardResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    /// Address of the product page on the website, without language prefix.
    pub path: String,
    pub short_description: String,
    /// `physical`, `digital`, `service` or `bundle`.
    pub kind: String,
    /// Lowest current price over the variants (no customer group, no code needed).
    pub price_from: Money,
    /// Highest current price over the variants: show "from" when it differs from `price_from`.
    pub price_to: Money,
    /// Price before the current reduction, to strike through; `null` without one.
    pub compare_at: Option<Money>,
    /// Price after an automatic discount (module `promotions`); `null` without one.
    pub sale_price: Option<Money>,
    /// When `sale_price` ends; `null`: no end set.
    pub sale_ends_at: Option<DateTime<Utc>>,
    /// Number of variants on sale.
    pub variant_count: i32,
    /// Average rating out of 5 (two decimals); `null` before the first rating.
    pub rating: Option<f64>,
    pub rating_count: i64,
    /// First image; `null` without one.
    pub image: Option<PublicImage>,
    pub seller: PublicSeller,
    /// Some text was written from an AI suggestion: label it (Vietnam's Law on Artificial
    /// Intelligence 134/2025/QH15).
    pub ai_assisted: bool,
    /// The product's options and how many values each has, e.g. `[{"name": "Size", "count": 3}]`
    /// ("3 sizes").
    pub variant_summary: Vec<OptionSummary>,
    /// Brand (the attribute with code `brand`); `null` without one.
    pub brand: Option<String>,
    /// Price per base unit of the cheapest variant with a net content (per kg, l, m or piece);
    /// `null` without one.
    pub unit_price: Option<UnitPrice>,
}

/// An option of a product and how many values it has.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogOptionSummary)]
pub(crate) struct OptionSummary {
    pub name: String,
    pub count: usize,
}

/// A price per base unit, e.g. 50 000 VND per kg.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogUnitPrice)]
pub(crate) struct UnitPrice {
    pub price: Money,
    /// `kg`, `l`, `m` or `piece`.
    pub per: String,
}

/// Dữ liệu đọc theo lô cho cả trang thẻ.
struct CardData {
    media: HashMap<Uuid, autoxide_media::api::MediaRef>,
    sellers: HashMap<Uuid, String>,
    options: HashMap<Uuid, Vec<crate::card::OptionCount>>,
    brands: HashMap<Uuid, String>,
}

/// Dựng thẻ sản phẩm từ dữ liệu đã đọc.
fn card(
    service: &CatalogService,
    site: &Site,
    card: &PublicCard,
    data: &CardData,
) -> Result<PublicCardResponse, Error> {
    let media = &data.media;
    let sellers = &data.sellers;
    let entry = &card.entry;
    let money = |minor: i64| {
        Money::from_columns(minor, &entry.currency)
            .map_err(|_| Error::internal("storefront currency is not a currency code"))
    };
    let owner = entry.owner_org_id;
    // Giá giảm đã hết (bảng dẫn xuất chưa kịp dựng lại): không hiện.
    let sale_running = entry.sale_ends_at.is_none_or(|ends| ends > Utc::now());
    Ok(PublicCardResponse {
        id: card.product.id,
        name: card.text.name.clone(),
        slug: card.text.slug.clone(),
        path: format!("/{}/{}", service.config.product_path_prefix, card.text.slug),
        short_description: card.text.short_description.clone(),
        kind: card.product.kind.clone(),
        price_from: money(entry.min_price_minor)?,
        price_to: money(entry.max_price_minor)?,
        compare_at: entry.compare_at_minor.map(money).transpose()?,
        sale_price: entry.sale_price_minor.filter(|_| sale_running).map(money).transpose()?,
        sale_ends_at: entry.sale_ends_at.filter(|_| sale_running),
        variant_count: entry.variant_count,
        rating: (entry.rating_count > 0).then(|| f64::from(entry.rating_x100) / 100.0),
        rating_count: entry.rating_count,
        image: card.image.as_ref().and_then(|row| image(row, media)),
        seller: PublicSeller {
            org_id: owner,
            name: sellers.get(&owner).cloned().unwrap_or_default(),
            own: site.lineage_ids().contains(&owner),
        },
        ai_assisted: card.product.ai_assisted,
        variant_summary: data
            .options
            .get(&card.product.id)
            .map(|options| {
                options
                    .iter()
                    .map(|option| OptionSummary { name: option.name.clone(), count: option.count })
                    .collect()
            })
            .unwrap_or_default(),
        brand: data.brands.get(&card.product.id).cloned(),
        unit_price: match (entry.unit_price_minor, &entry.unit_price_unit) {
            (Some(minor), Some(per)) => Some(UnitPrice { price: money(minor)?, per: per.clone() }),
            _ => None,
        },
    })
}

/// Thẻ cho nhiều sản phẩm: một lần đọc ảnh và tên người bán cho cả trang.
pub(crate) async fn cards(
    service: &CatalogService,
    tenant: &TenantContext,
    site: &Site,
    locale: &str,
    rows: &[PublicCard],
) -> Result<Vec<PublicCardResponse>, Error> {
    let products: Vec<Uuid> = rows.iter().map(|row| row.product.id).collect();
    let options = service.option_counts(tenant, &products, locale).await?;
    cards_with(service, tenant, site, locale, rows, options).await
}

/// Như [`cards`], với số giá trị tuỳ chọn đã có (trang sản phẩm đã đọc tuỳ chọn, không đọc lại —
/// D-20261120-step39-hot-paths).
async fn cards_with(
    service: &CatalogService,
    tenant: &TenantContext,
    site: &Site,
    locale: &str,
    rows: &[PublicCard],
    options: HashMap<Uuid, Vec<crate::card::OptionCount>>,
) -> Result<Vec<PublicCardResponse>, Error> {
    let images: Vec<Uuid> =
        rows.iter().filter_map(|row| row.image.as_ref().map(|image| image.media_id)).collect();
    let owners: Vec<Uuid> = rows.iter().map(|row| row.entry.owner_org_id).collect();
    let products: Vec<Uuid> = rows.iter().map(|row| row.product.id).collect();
    let data = CardData {
        media: service.public_media(tenant, &images).await?,
        sellers: service.seller_names(tenant, &owners).await?,
        options,
        brands: service.brands(tenant, &products, locale).await?,
    };
    rows.iter().map(|row| card(service, site, row, &data)).collect()
}

/// Order of the public product list.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CatalogPublicSort)]
pub(crate) enum PublicSortDto {
    /// Most recently put on sale first (the default; `order=asc` for oldest first).
    Newest,
    /// Cheapest first (sets `order=asc`).
    PriceAsc,
    /// Most expensive first (sets `order=desc`).
    PriceDesc,
    /// Best sellers first.
    BestSelling,
    /// Best rated first.
    Rating,
}

/// Whose products to list.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CatalogPublicSoldBy)]
pub(crate) enum SoldByDto {
    /// Products of the website's own unit or of a unit above it.
    Own,
    /// Products of sellers below the website (a marketplace).
    Marketplace,
}

/// Filters of the public product list. Attribute filters are extra parameters
/// `attr.<code>=<value>` (several values separated by `,` match any of them): only attributes marked
/// filterable; an option is given by its code, a decimal as text (`12.5`), a date as `YYYY-MM-DD`.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct PublicProductsQuery {
    /// Language of the texts; only products written in it are listed.
    #[param(example = "vi")]
    pub locale: String,
    /// Only products in the category with this slug (in `locale`) or one below it.
    pub category: Option<String>,
    /// Lowest `price_from`, in the currency's smallest unit.
    pub price_min: Option<i64>,
    /// Highest `price_from`, in the currency's smallest unit.
    pub price_max: Option<i64>,
    /// Only products whose name, SKU or short description contains this text, with or without
    /// Vietnamese accents. At most 200 characters.
    pub q: Option<String>,
    /// Order of the list; keep it while following `next_cursor`.
    #[param(inline)]
    pub sort: Option<PublicSortDto>,
    /// `true`: only products with an automatic discount running now.
    pub on_sale: Option<bool>,
    /// Only products of this seller (organization unit) or of the units below it.
    pub seller: Option<Uuid>,
    /// Only the website's own products, or only marketplace sellers' products.
    #[param(inline)]
    pub sold_by: Option<SoldByDto>,
    /// Lowest average rating, 1 to 5.
    #[param(minimum = 1, maximum = 5)]
    pub min_rating: Option<u8>,
}

/// List products on sale (public).
///
/// No sign-in needed. Products on sale and shown on the website asking (its own, those of the
/// units above it, and those of sellers below it), with a text in `locale`. Filter by category,
/// attributes (`attr.<code>=<value>`), price range, text, seller and rating; prices are the
/// current prices of the website's price list, without customer group.
#[utoipa::path(
    get,
    path = "/public/products",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_catalog_products",
    security((), ("bearer" = [])),
    params(PublicProductsQuery, PageQuery, SiteQuery),
    responses(
        (status = 200, description = "One page of products.", body = Page<PublicCardResponse>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "The `site` is not a unit of this tenant; code `site_not_found`.", body = Problem),
        (status = 422, description = "A parameter is invalid (unknown language, category or attribute, text too long); see `errors`.", body = Problem),
    )
)]
async fn list_public_products(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Query(query): Query<PublicProductsQuery>,
    Query(mut paging): Query<PageQuery>,
    Query(raw): Query<Vec<(String, String)>>,
) -> Result<Response, Error> {
    let filter = parse_filter(&service, query, raw, &mut paging)?;
    let page = PageRequest::try_from(paging)?;
    let Page { data, pagination } = service.public_products(&tenant, &site, &filter, &page).await?;
    let data = cards(&service, &tenant, &site, &filter.locale, &data).await?;
    cached(&tenant, Page { data, pagination })
}

/// Đọc và kiểm bộ lọc của danh sách công khai (dùng chung cho danh sách và số đếm facet); gom
/// lỗi một lần. `sort=price_asc|price_desc` đặt luôn `order` của `paging`.
pub(crate) fn parse_filter(
    service: &CatalogService,
    query: PublicProductsQuery,
    raw: Vec<(String, String)>,
    paging: &mut PageQuery,
) -> Result<PublicFilter, Error> {
    let locale = service.locale(&query.locale)?;
    let mut problems = Vec::new();
    if query.q.as_deref().is_some_and(|q| q.trim().chars().count() > QUERY_MAX) {
        problems.push(
            FieldError::new("q", "length", "must be at most {max} characters")
                .with_param("max", QUERY_MAX),
        );
    }
    let attributes: Vec<(String, Vec<String>)> = raw
        .into_iter()
        .filter_map(|(name, value)| {
            let code = name.strip_prefix("attr.")?.to_owned();
            let values = value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned)
                .collect();
            Some((code, values))
        })
        .collect();
    if attributes.len() > MAX_ATTRIBUTE_FILTERS {
        problems.push(
            FieldError::new("attr", "count", "at most {max} attribute filters")
                .with_param("max", MAX_ATTRIBUTE_FILTERS),
        );
    }
    if query.min_rating.is_some_and(|rating| !(1..=5).contains(&rating)) {
        problems.push(FieldError::new("min_rating", "range", "must be 1 to 5"));
    }
    let sort = match query.sort {
        None | Some(PublicSortDto::Newest) => PublicSort::Newest,
        Some(PublicSortDto::PriceAsc) => {
            paging.order = Some("asc".to_owned());
            PublicSort::PriceAsc
        }
        Some(PublicSortDto::PriceDesc) => {
            paging.order = Some("desc".to_owned());
            PublicSort::PriceDesc
        }
        Some(PublicSortDto::BestSelling) => PublicSort::BestSelling,
        Some(PublicSortDto::Rating) => PublicSort::Rating,
    };
    if !problems.is_empty() {
        return Err(Error::validation(problems));
    }
    let filter = PublicFilter {
        locale,
        category: query.category,
        attributes,
        price_min: query.price_min,
        price_max: query.price_max,
        q: query.q,
        sort,
        on_sale: query.on_sale.unwrap_or(false),
        seller: query.seller,
        sold_by: query.sold_by.map(|sold_by| match sold_by {
            SoldByDto::Own => SoldBy::Own,
            SoldByDto::Marketplace => SoldBy::Marketplace,
        }),
        min_rating: query.min_rating.map(|rating| i32::from(rating) * 100),
        ..Default::default()
    };
    Ok(filter)
}

/// A choice of a product option, in the language asked.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicOption)]
pub(crate) struct PublicOption {
    pub id: Uuid,
    pub name: String,
    pub values: Vec<PublicOptionValue>,
}

/// A value of a product option.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicOptionValue)]
pub(crate) struct PublicOptionValue {
    pub id: Uuid,
    pub label: String,
}

/// A variant on sale.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicVariant)]
pub(crate) struct PublicVariant {
    pub id: Uuid,
    pub sku: String,
    /// GTIN / EAN barcode, for structured data.
    pub barcode: Option<String>,
    /// One value of each option, in the options' order.
    pub option_value_ids: Vec<Uuid>,
    /// Current price; `null`: not sold on this website now.
    pub price: Option<Money>,
    /// Price before the current reduction; `null` without one.
    pub compare_at: Option<Money>,
    /// When `price` is a temporary reduced price: the price in effect just before it (the price to
    /// strike through in structured data); `null` otherwise.
    pub regular_price: Option<Money>,
    /// When `price` ends (a temporary price), for `priceValidUntil`; `null`: no end set.
    pub price_valid_until: Option<DateTime<Utc>>,
    /// Price per base unit when the variant has a net content; `null` otherwise.
    pub unit_price: Option<UnitPrice>,
    /// Address of the product page with this variant chosen (for each `Offer` of a
    /// `ProductGroup`).
    pub path: String,
    /// `in_stock`, `backorder` or `preorder`.
    pub sale_mode: String,
    /// For `preorder`: the expected date it is available.
    pub preorder_available_on: Option<NaiveDate>,
    /// For `preorder`: the message to show, in the language asked.
    pub preorder_message: Option<String>,
    pub image_media_id: Option<Uuid>,
    /// What is particular to this variant (how many it serves, what is in the box, care…), in the
    /// language asked; empty when the variant has none.
    pub attributes: Vec<PublicAttributeResponse>,
}

impl From<crate::public::PublicAttribute> for PublicAttributeResponse {
    fn from(item: crate::public::PublicAttribute) -> Self {
        PublicAttributeResponse {
            code: item.code,
            name: item.name,
            unit: item.unit,
            value: item.value,
            label: item.label,
        }
    }
}

/// An attribute of the product, in the language asked.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicAttribute)]
pub(crate) struct PublicAttributeResponse {
    pub code: String,
    pub name: String,
    pub unit: Option<String>,
    /// As written: text, a whole number, a decimal as text, `true` / `false`, a date, an option
    /// code or a list of option codes.
    pub value: Value,
    /// Ready to show, in the language asked.
    pub label: String,
}

/// A category, in the language asked.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicCategory)]
pub(crate) struct PublicCategoryResponse {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub name: String,
    pub slug: String,
    /// Depth in the tree (top level is 0).
    pub depth: i32,
    pub position: i32,
}

/// The same product in another language.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicTranslation)]
pub(crate) struct PublicTranslation {
    pub locale: String,
    pub slug: String,
    pub path: String,
}

/// A product page.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPublicProduct)]
pub(crate) struct PublicProductResponse {
    #[serde(flatten)]
    pub card: PublicCardResponse,
    pub description: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub warranty_months: Option<i32>,
    pub options: Vec<PublicOption>,
    pub variants: Vec<PublicVariant>,
    pub images: Vec<PublicImage>,
    pub attributes: Vec<PublicAttributeResponse>,
    pub categories: Vec<PublicCategoryResponse>,
    /// Other languages of the product, for `hreflang`.
    pub translations: Vec<PublicTranslation>,
}

/// The language of a public read.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct LocaleQuery {
    /// Language of the texts.
    #[param(example = "vi")]
    pub locale: String,
}

/// Dữ liệu đọc thêm cho trang sản phẩm.
struct PageData {
    media: HashMap<Uuid, autoxide_media::api::MediaRef>,
    /// Giá ngay trước giá giảm tạm thời, theo biến thể.
    regular: HashMap<Uuid, crate::api::PriceSnapshot>,
    alts: AltData,
    /// Thuộc tính theo biến thể, ở ngôn ngữ của trang.
    variant_attributes: HashMap<Uuid, Vec<crate::public::PublicAttribute>>,
}

/// `alt` theo ngôn ngữ của trang sản phẩm.
struct AltData {
    overrides: HashMap<Uuid, String>,
    is_default_locale: bool,
}

fn product_page(
    service: &CatalogService,
    product: PublicProduct,
    card: PublicCardResponse,
    data: &PageData,
    locale: &str,
) -> PublicProductResponse {
    let media = &data.media;
    let money = |minor: i64, currency: &str| Money::from_columns(minor, currency).ok();
    let prefix = &service.config.product_path_prefix;
    let text: &translation::Model = &product.card.text;
    PublicProductResponse {
        description: text.description.clone(),
        meta_title: text.meta_title.clone(),
        meta_description: text.meta_description.clone(),
        warranty_months: product.card.product.warranty_months,
        options: product
            .options
            .iter()
            .map(|detail| PublicOption {
                id: detail.option.id,
                name: pick(&detail.option.names, locale),
                values: detail
                    .values
                    .iter()
                    .map(|value| PublicOptionValue {
                        id: value.id,
                        label: pick(&value.labels, locale),
                    })
                    .collect(),
            })
            .collect(),
        variants: product
            .variants
            .iter()
            .map(|row| {
                let price = product.prices.get(&row.id);
                let messages: BTreeMap<String, String> = row
                    .preorder_messages
                    .as_deref()
                    .and_then(|raw| serde_json::from_str(raw).ok())
                    .unwrap_or_default();
                PublicVariant {
                    id: row.id,
                    sku: row.sku.clone(),
                    barcode: row.barcode.clone(),
                    option_value_ids: row
                        .option_key
                        .split(',')
                        .filter_map(|id| id.parse().ok())
                        .collect(),
                    price: price.map(|price| price.amount),
                    compare_at: price.and_then(|price| price.compare_at),
                    regular_price: data.regular.get(&row.id).map(|before| before.amount),
                    price_valid_until: price.and_then(|price| price.valid_to),
                    unit_price: price.and_then(|price| {
                        let (minor, per) = crate::variants::unit_price(
                            price.amount.minor(),
                            row.net_quantity?,
                            row.net_unit.as_deref()?,
                        )?;
                        Some(UnitPrice {
                            price: Money::from_columns(minor, price.amount.currency().code())
                                .ok()?,
                            per: per.to_owned(),
                        })
                    }),
                    path: format!("/{prefix}/{}?variant={}", text.slug, row.id),
                    sale_mode: row.sale_mode.clone(),
                    preorder_available_on: row.preorder_available_on,
                    preorder_message: messages.get(locale).cloned(),
                    image_media_id: row.image_media_id,
                    attributes: data
                        .variant_attributes
                        .get(&row.id)
                        .into_iter()
                        .flatten()
                        .cloned()
                        .map(PublicAttributeResponse::from)
                        .collect(),
                }
            })
            .collect(),
        images: product
            .media
            .iter()
            .filter_map(|row| {
                let mut found = image(row, media)?;
                let media_alt = media.get(&row.media_id).map_or("", |item| item.alt.as_str());
                found.alt = crate::media_alts::public_alt(
                    row,
                    &data.alts.overrides,
                    data.alts.is_default_locale,
                    media_alt,
                );
                Some(found)
            })
            .collect(),
        attributes: product.attributes.into_iter().map(PublicAttributeResponse::from).collect(),
        categories: product
            .categories
            .into_iter()
            .map(|(row, text)| PublicCategoryResponse {
                id: row.id,
                parent_id: row.parent_id,
                name: text.name,
                slug: text.slug,
                depth: row.depth,
                position: row.position,
            })
            .collect(),
        translations: product
            .translations
            .iter()
            .filter(|other| other.locale != text.locale)
            .map(|other| PublicTranslation {
                locale: other.locale.clone(),
                slug: other.slug.clone(),
                path: format!("/{prefix}/{}", other.slug),
            })
            .collect(),
        card: PublicCardResponse {
            compare_at: card.compare_at.or_else(|| {
                product
                    .card
                    .entry
                    .compare_at_minor
                    .and_then(|minor| money(minor, &product.card.entry.currency))
            }),
            ..card
        },
    }
}

/// Get a product page by its address (public).
///
/// No sign-in needed. The product, its variants with current prices, images, attributes,
/// categories and other languages — enough for the page and its structured data
/// (`Product` / `ProductGroup` / `Offer`). Only a product on sale on the website asking.
#[utoipa::path(
    get,
    path = "/public/products/by-slug/{slug}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_catalog_product",
    security((), ("bearer" = [])),
    params(("slug" = String, Path, description = "Slug of the product in `locale`."), LocaleQuery, SiteQuery),
    responses(
        (status = 200, description = "The product.", body = Data<PublicProductResponse>, headers(("ETag" = String, description = "Fingerprint of the whole answer (prices and images included); send it in `If-None-Match` to get 304."))),
        (status = 404, description = "No product on sale here has this slug in this language; code `catalog.product_slug_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported; code `catalog.unsupported_locale`.", body = Problem),
    )
)]
async fn get_public_product(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Path(slug): Path<String>,
    Query(query): Query<LocaleQuery>,
) -> Result<Response, Error> {
    let locale = service.locale(&query.locale)?;
    let product = service.public_product(&tenant, &site, &locale, &slug).await?;
    let ids: Vec<Uuid> = product.media.iter().map(|row| row.media_id).collect();
    // `alt` theo ngôn ngữ của trang (D-202): media theo ngôn ngữ, bản ghi đè của sản phẩm.
    let media = service.public_media_in(&tenant, &ids, &locale).await?;
    // Bản ghi đè `alt` và tuỳ chọn đã đọc cùng sản phẩm: lọc lại, không đọc lần hai.
    let alts = AltData {
        overrides: product
            .media_alts
            .iter()
            .filter(|row| row.locale == locale)
            .map(|row| (row.media_id, row.alt.clone()))
            .collect(),
        is_default_locale: product.card.product.default_locale == locale,
    };
    let options = HashMap::from([(
        product.card.product.id,
        crate::card::option_counts_of(&product.options, &locale),
    )]);
    let mut rows =
        cards_with(&service, &tenant, &site, &locale, std::slice::from_ref(&product.card), options)
            .await?;
    let pricing = autoxide_kernel::tenancy::OrgId::from_uuid(product.card.entry.org_id);
    let regular = service.regular_prices(&tenant, pricing, &product.prices).await?;
    let card = rows.pop().ok_or_else(|| crate::errors::product_slug_not_found(&slug))?;
    let variant_ids: Vec<Uuid> = product.variants.iter().map(|row| row.id).collect();
    let variant_attributes =
        service.public_variant_attributes(&tenant, &variant_ids, &locale).await?;
    let data = PageData { media, regular, alts, variant_attributes };
    let page = product_page(&service, product, card, &data, &locale);
    cached(&tenant, Data { data: page })
}

/// List the category tree (public).
///
/// No sign-in needed. Visible categories with a name in `locale`, parents before children, by
/// depth then position; a category under a hidden one is left out.
#[utoipa::path(
    get,
    path = "/public/categories",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_catalog_categories",
    security((), ("bearer" = [])),
    params(LocaleQuery),
    responses(
        (status = 200, description = "Every visible category.", body = Data<Vec<PublicCategoryResponse>>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 422, description = "The language is not supported; code `catalog.unsupported_locale`.", body = Problem),
    )
)]
async fn list_public_categories(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    _principal: Option<Principal>,
    Query(query): Query<LocaleQuery>,
) -> Result<Response, Error> {
    let locale = service.locale(&query.locale)?;
    let rows = service.public_categories(&tenant, &locale).await?;
    let data: Vec<PublicCategoryResponse> = rows
        .into_iter()
        .map(|row| PublicCategoryResponse {
            id: row.category.id,
            parent_id: row.category.parent_id,
            name: row.text.name,
            slug: row.text.slug,
            depth: row.category.depth,
            position: row.category.position,
        })
        .collect();
    cached(&tenant, Data { data })
}

/// Get a category by its address (public).
///
/// No sign-in needed. List its products with `GET /public/products?category=<slug>`.
#[utoipa::path(
    get,
    path = "/public/categories/{slug}",
    extensions(("x-public" = json!(true))),
    operation_id = "get_public_catalog_category",
    security((), ("bearer" = [])),
    params(("slug" = String, Path, description = "Slug of the category in `locale`."), LocaleQuery),
    responses(
        (status = 200, description = "The category.", body = Data<PublicCategoryResponse>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 404, description = "No visible category has this slug in this language; code `catalog.category_slug_not_found`.", body = Problem),
        (status = 422, description = "The language is not supported; code `catalog.unsupported_locale`.", body = Problem),
    )
)]
async fn get_public_category(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    _principal: Option<Principal>,
    Path(slug): Path<String>,
    Query(query): Query<LocaleQuery>,
) -> Result<Response, Error> {
    let locale = service.locale(&query.locale)?;
    let row = service.public_category(&tenant, &locale, &slug).await?;
    let data = PublicCategoryResponse {
        id: row.category.id,
        parent_id: row.category.parent_id,
        name: row.text.name,
        slug: row.text.slug,
        depth: row.category.depth,
        position: row.category.position,
    };
    cached(&tenant, Data { data })
}

/// The language of a sitemap.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SitemapQuery {
    /// Only addresses in this language; omitted: every language.
    pub locale: Option<String>,
}

/// A product in the sitemap.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogSitemapItem)]
pub(crate) struct SitemapItemResponse {
    pub id: Uuid,
    /// Address of the product in each of its languages.
    pub translations: Vec<PublicTranslation>,
    /// Last change of the product, its texts or its price on this website.
    pub modified_at: DateTime<Utc>,
}

/// List product addresses for the sitemap (public).
///
/// No sign-in needed. Every product on sale on the website asking, with its address in each
/// language; follow `next_cursor` (at most 200 per page).
#[utoipa::path(
    get,
    path = "/public/sitemap",
    extensions(("x-public" = json!(true))),
    operation_id = "list_public_catalog_sitemap",
    security((), ("bearer" = [])),
    params(SitemapQuery, PageQuery, SiteQuery),
    responses(
        (status = 200, description = "One page of addresses.", body = Page<SitemapItemResponse>, headers(("ETag" = String, description = "Fingerprint of the whole answer; send it in `If-None-Match` to get 304."))),
        (status = 422, description = "A parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn public_sitemap(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    site: Site,
    _principal: Option<Principal>,
    Query(query): Query<SitemapQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Response, Error> {
    let locale = query.locale.as_deref().map(|locale| service.locale(locale)).transpose()?;
    let page = PageRequest::try_from(paging)?;
    let Page { data, pagination } =
        service.public_sitemap(&tenant, &site, locale.as_deref(), &page).await?;
    let prefix = &service.config.product_path_prefix;
    let data = data
        .into_iter()
        .map(|item| SitemapItemResponse {
            id: item.entry.product_id,
            translations: item
                .translations
                .into_iter()
                .map(|text| PublicTranslation {
                    path: format!("/{prefix}/{}", text.slug),
                    locale: text.locale,
                    slug: text.slug,
                })
                .collect(),
            modified_at: item.modified_at,
        })
        .collect();
    cached(&tenant, Page { data, pagination })
}
