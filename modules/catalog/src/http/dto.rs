//! Kiểu dữ liệu vào / ra của REST và MCP (cùng kiểu cho hai đường). Doc comment `///` của kiểu
//! `ToSchema` vào tài liệu OpenAPI: viết tiếng Anh cho người gọi API.

use std::collections::BTreeMap;

use autoxide_kernel::money::Money;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::api::{BundleComponent, FloorAction, ProductKind, ProductStatus, SaleMode};
use crate::categories::{CategoryDetail, CategoryText};
use crate::entity::{price, price_guard, price_list, product_media, translation, variant};
use crate::products::{ProductDetail, ProductSummary, TextInput, parse_kind, parse_status};
use crate::service::texts_of;
use crate::variants::{NewVariant, OptionDetail, parse_sale_mode};

/// Một trường có mặt trong thân (kể cả `null`) → `Some(…)`; vắng → `None`.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Kind of product: `physical` (shipped, stocked), `digital` (downloaded), `service` (not
/// shipped), `bundle` (takes stock from each component).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CatalogKind)]
pub(crate) enum KindDto {
    Physical,
    Digital,
    Service,
    Bundle,
}

impl From<KindDto> for ProductKind {
    fn from(kind: KindDto) -> Self {
        match kind {
            KindDto::Physical => ProductKind::Physical,
            KindDto::Digital => ProductKind::Digital,
            KindDto::Service => ProductKind::Service,
            KindDto::Bundle => ProductKind::Bundle,
        }
    }
}

impl From<ProductKind> for KindDto {
    fn from(kind: ProductKind) -> Self {
        match kind {
            ProductKind::Physical => KindDto::Physical,
            ProductKind::Digital => KindDto::Digital,
            ProductKind::Service => KindDto::Service,
            ProductKind::Bundle => KindDto::Bundle,
        }
    }
}

/// Sale status: `draft`, `active` (on sale), `inactive` (stopped), `archived`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CatalogStatus)]
pub(crate) enum StatusDto {
    Draft,
    Active,
    Inactive,
    Archived,
}

impl From<StatusDto> for ProductStatus {
    fn from(status: StatusDto) -> Self {
        match status {
            StatusDto::Draft => ProductStatus::Draft,
            StatusDto::Active => ProductStatus::Active,
            StatusDto::Inactive => ProductStatus::Inactive,
            StatusDto::Archived => ProductStatus::Archived,
        }
    }
}

impl From<ProductStatus> for StatusDto {
    fn from(status: ProductStatus) -> Self {
        match status {
            ProductStatus::Draft => StatusDto::Draft,
            ProductStatus::Active => StatusDto::Active,
            ProductStatus::Inactive => StatusDto::Inactive,
            ProductStatus::Archived => StatusDto::Archived,
        }
    }
}

/// How a variant sells: `in_stock` (only while in stock), `backorder` (also when out of stock),
/// `preorder` (before it arrives, see `preorder_available_on`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CatalogSaleMode)]
pub(crate) enum SaleModeDto {
    InStock,
    Backorder,
    Preorder,
}

impl From<SaleModeDto> for SaleMode {
    fn from(mode: SaleModeDto) -> Self {
        match mode {
            SaleModeDto::InStock => SaleMode::InStock,
            SaleModeDto::Backorder => SaleMode::Backorder,
            SaleModeDto::Preorder => SaleMode::Preorder,
        }
    }
}

impl From<SaleMode> for SaleModeDto {
    fn from(mode: SaleMode) -> Self {
        match mode {
            SaleMode::InStock => SaleModeDto::InStock,
            SaleMode::Backorder => SaleModeDto::Backorder,
            SaleMode::Preorder => SaleModeDto::Preorder,
        }
    }
}

/// Texts of a product in one language.
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogTextInput)]
pub(crate) struct TextRequest {
    /// 1 to 200 characters.
    #[schema(example = "Cotton T-shirt")]
    pub name: String,
    /// Last part of the public address; omitted: made from the name. Unique per language in the
    /// tenant (`catalog.slug_taken`).
    pub slug: Option<String>,
    /// Up to 1000 characters.
    #[serde(default)]
    pub short_description: String,
    /// Plain text up to 50000 characters; blank lines separate paragraphs and clients may render
    /// minimal Markdown. HTML is not interpreted.
    #[serde(default)]
    pub description: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
}

impl From<TextRequest> for TextInput {
    fn from(text: TextRequest) -> Self {
        TextInput {
            name: text.name,
            slug: text.slug,
            short_description: text.short_description,
            description: text.description,
            meta_title: text.meta_title,
            meta_description: text.meta_description,
            slug_generated: false,
        }
    }
}

/// A variant to create, or the first variant of a new product.
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogVariantInput)]
pub(crate) struct VariantRequest {
    /// Stock keeping unit, 1 to 64 characters; unique in the tenant ignoring case and spaces
    /// (`catalog.sku_taken`).
    #[schema(example = "TSHIRT-RED-M")]
    pub sku: String,
    /// Barcode (GTIN or own code), up to 32 letters, digits or `-`; GTIN-8/12/13/14 must have a
    /// right check digit. Unique in the tenant (`catalog.barcode_taken`).
    pub barcode: Option<String>,
    /// One value id of each option of the product; empty when the product has no options.
    #[serde(default)]
    pub option_value_ids: Vec<Uuid>,
    /// On by default.
    pub active: Option<bool>,
    /// Default `in_stock`; digital products and services are always `in_stock`.
    #[schema(inline)]
    pub sale_mode: Option<SaleModeDto>,
    /// Expected availability date of a pre-order, after today
    /// (`catalog.preorder_date_invalid`).
    pub preorder_available_on: Option<NaiveDate>,
    /// Total pre-order units, above 0 (`catalog.preorder_limit_invalid`); omitted: no limit.
    pub preorder_limit: Option<i64>,
    /// Pre-orders stop being taken at this time.
    pub preorder_ends_at: Option<DateTime<Utc>>,
    /// Public pre-order note per language, e.g. `{"en": "Ships from 15 December"}`.
    #[serde(default)]
    pub preorder_messages: BTreeMap<String, String>,
    /// Shipping weight in grams (physical products and bundles).
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    /// Main image of the variant (public media).
    pub image_media_id: Option<Uuid>,
}

impl From<VariantRequest> for NewVariant {
    fn from(input: VariantRequest) -> Self {
        NewVariant {
            sku: input.sku,
            barcode: input.barcode,
            option_value_ids: input.option_value_ids,
            active: input.active,
            sale_mode: input.sale_mode.map(SaleMode::from),
            preorder_available_on: input.preorder_available_on,
            preorder_limit: input.preorder_limit,
            preorder_ends_at: input.preorder_ends_at,
            preorder_messages: input.preorder_messages,
            weight_grams: input.weight_grams,
            length_mm: input.length_mm,
            width_mm: input.width_mm,
            height_mm: input.height_mm,
            image_media_id: input.image_media_id,
        }
    }
}

/// A new product, created as a `draft` with its first variant.
#[derive(Debug, Clone, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCreateProduct)]
pub(crate) struct CreateProductRequest {
    /// The organization unit that owns the product; you need `catalog.products.write` there.
    /// Omit when you work in one unit only (or for the whole tenant).
    pub org_id: Option<Uuid>,
    #[schema(inline)]
    pub kind: KindDto,
    /// Tax category code of the tax engine, e.g. `vat.standard` (`GET /api/v1/tax/categories`).
    #[schema(example = "vat.standard")]
    pub tax_category: String,
    /// Language of `text`; becomes the default language of the product.
    #[schema(example = "vi")]
    pub locale: String,
    #[schema(inline)]
    pub text: TextRequest,
    /// Warranty in months, 1 to 600; omitted: no warranty.
    pub warranty_months: Option<i32>,
    #[serde(default)]
    pub lot_tracked: bool,
    #[serde(default)]
    pub serial_tracked: bool,
    /// Categories of the product; the first is the main one.
    #[serde(default)]
    pub category_ids: Vec<Uuid>,
    #[schema(inline)]
    pub variant: VariantRequest,
}

/// Changes to a product. Omitted fields stay as they are.
#[derive(Debug, Clone, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogUpdateProduct)]
pub(crate) struct UpdateProductRequest {
    pub tax_category: Option<String>,
    /// `null` removes the warranty.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i32>, nullable)]
    pub warranty_months: Option<Option<i32>>,
    pub lot_tracked: Option<bool>,
    pub serial_tracked: Option<bool>,
    /// A language the product already has texts in.
    pub default_locale: Option<String>,
}

/// Texts of a product in one language.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogTranslation)]
pub(crate) struct TranslationResponse {
    pub locale: String,
    pub name: String,
    pub slug: String,
    /// Public address of the product in this language, computed by the server:
    /// `/<AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX>/<slug>`, without a language prefix.
    #[schema(example = "/products/cotton-t-shirt")]
    pub path: String,
    pub short_description: String,
    pub description: String,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
}

impl TranslationResponse {
    fn new(row: &translation::Model, prefix: &str) -> Self {
        TranslationResponse {
            locale: row.locale.clone(),
            name: row.name.clone(),
            slug: row.slug.clone(),
            path: format!("/{prefix}/{}", row.slug),
            short_description: row.short_description.clone(),
            description: row.description.clone(),
            meta_title: row.meta_title.clone(),
            meta_description: row.meta_description.clone(),
        }
    }
}

/// A value of an option, e.g. "Red".
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CatalogOptionValue)]
pub(crate) struct OptionValueDto {
    /// Keep the id of an existing value so variants keep pointing at it; omit for a new value.
    pub id: Option<Uuid>,
    /// Label per language, e.g. `{"en": "Red"}`.
    pub labels: BTreeMap<String, String>,
}

/// An option of a product, e.g. colour or size.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(as = CatalogOption)]
pub(crate) struct OptionDto {
    /// Keep the id of an existing option; omit for a new one.
    pub id: Option<Uuid>,
    /// Name per language, e.g. `{"en": "Colour"}`.
    pub names: BTreeMap<String, String>,
    pub values: Vec<OptionValueDto>,
}

impl From<&OptionDetail> for OptionDto {
    fn from(detail: &OptionDetail) -> Self {
        OptionDto {
            id: Some(detail.option.id),
            names: texts_of(&detail.option.names),
            values: detail
                .values
                .iter()
                .map(|value| OptionValueDto { id: Some(value.id), labels: texts_of(&value.labels) })
                .collect(),
        }
    }
}

/// A variant of a product.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogVariant)]
pub(crate) struct VariantResponse {
    pub id: Uuid,
    pub product_id: Uuid,
    pub sku: String,
    pub barcode: Option<String>,
    pub option_value_ids: Vec<Uuid>,
    pub position: i32,
    pub active: bool,
    pub sale_mode: SaleModeDto,
    pub preorder_available_on: Option<NaiveDate>,
    pub preorder_limit: Option<i64>,
    pub preorder_ends_at: Option<DateTime<Utc>>,
    pub preorder_messages: BTreeMap<String, String>,
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    pub image_media_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<&variant::Model> for VariantResponse {
    fn from(row: &variant::Model) -> Self {
        VariantResponse {
            id: row.id,
            product_id: row.product_id,
            sku: row.sku.clone(),
            barcode: row.barcode.clone(),
            option_value_ids: row
                .option_key
                .split(',')
                .filter_map(|id| Uuid::parse_str(id).ok())
                .collect(),
            position: row.position,
            active: row.active,
            sale_mode: parse_sale_mode(&row.sale_mode).into(),
            preorder_available_on: row.preorder_available_on,
            preorder_limit: row.preorder_limit,
            preorder_ends_at: row.preorder_ends_at,
            preorder_messages: row.preorder_messages.as_deref().map(texts_of).unwrap_or_default(),
            weight_grams: row.weight_grams,
            length_mm: row.length_mm,
            width_mm: row.width_mm,
            height_mm: row.height_mm,
            image_media_id: row.image_media_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// An image of a product.
#[derive(Debug, Clone, Serialize, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogProductMedia)]
pub(crate) struct ProductMediaDto {
    /// Public media of the `media` module (`catalog.media_not_public` otherwise).
    pub media_id: Uuid,
    /// The image shows this variant.
    pub variant_id: Option<Uuid>,
    /// Alternative text in the product's default language; empty: the media's own `alt`.
    #[serde(default)]
    pub alt: String,
}

impl From<&product_media::Model> for ProductMediaDto {
    fn from(row: &product_media::Model) -> Self {
        ProductMediaDto { media_id: row.media_id, variant_id: row.variant_id, alt: row.alt.clone() }
    }
}

/// A product with everything it has.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogProduct)]
pub(crate) struct ProductResponse {
    pub id: Uuid,
    /// The organization unit that owns the product.
    pub org_id: Uuid,
    pub kind: KindDto,
    pub status: StatusDto,
    pub tax_category: String,
    pub primary_category_id: Option<Uuid>,
    pub category_ids: Vec<Uuid>,
    /// `true` for physical products: stock is counted.
    pub track_inventory: bool,
    pub lot_tracked: bool,
    pub serial_tracked: bool,
    pub warranty_months: Option<i32>,
    /// Can the product go on sale: `ready`, `missing_attributes`, `missing_documents`,
    /// `restricted`.
    pub readiness: String,
    pub default_locale: String,
    /// Some texts were written with AI help (Law 134/2025/QH15).
    pub ai_assisted: bool,
    /// The product was on sale once: it can only be archived, not deleted.
    pub ever_active: bool,
    pub translations: Vec<TranslationResponse>,
    pub options: Vec<OptionDto>,
    pub variants: Vec<VariantResponse>,
    pub media: Vec<ProductMediaDto>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
}

impl From<ProductDetail> for ProductResponse {
    fn from(detail: ProductDetail) -> Self {
        let product = detail.product;
        ProductResponse {
            id: product.id,
            org_id: product.org_id,
            kind: parse_kind(&product.kind).into(),
            status: parse_status(&product.status).into(),
            tax_category: product.tax_category,
            primary_category_id: product.primary_category_id,
            category_ids: detail.category_ids,
            track_inventory: product.track_inventory,
            lot_tracked: product.lot_tracked,
            serial_tracked: product.serial_tracked,
            warranty_months: product.warranty_months,
            readiness: product.readiness,
            default_locale: product.default_locale,
            ai_assisted: product.ai_assisted,
            ever_active: product.ever_active,
            translations: detail
                .translations
                .iter()
                .map(|row| TranslationResponse::new(row, &detail.path_prefix))
                .collect(),
            options: detail.options.iter().map(OptionDto::from).collect(),
            variants: detail.variants.iter().map(VariantResponse::from).collect(),
            media: detail.media.iter().map(ProductMediaDto::from).collect(),
            created_at: product.created_at,
            updated_at: product.updated_at,
            activated_at: product.activated_at,
        }
    }
}

/// A product in a list.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogProductListItem)]
pub(crate) struct ProductListItem {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: KindDto,
    pub status: StatusDto,
    pub readiness: String,
    /// Name in the product's default language.
    pub name: String,
    pub slug: String,
    pub default_locale: String,
    pub variant_count: u64,
    pub updated_at: DateTime<Utc>,
}

impl From<ProductSummary> for ProductListItem {
    fn from(summary: ProductSummary) -> Self {
        let product = summary.product;
        ProductListItem {
            id: product.id,
            org_id: product.org_id,
            kind: parse_kind(&product.kind).into(),
            status: parse_status(&product.status).into(),
            readiness: product.readiness,
            name: summary.text.as_ref().map(|text| text.name.clone()).unwrap_or_default(),
            slug: summary.text.as_ref().map(|text| text.slug.clone()).unwrap_or_default(),
            default_locale: product.default_locale,
            variant_count: summary.variant_count,
            updated_at: product.updated_at,
        }
    }
}

/// A component of a bundle.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogComponent)]
pub(crate) struct ComponentDto {
    pub variant_id: Uuid,
    /// Units of the component in one bundle, 1 to 10000.
    pub quantity: i64,
}

impl From<ComponentDto> for BundleComponent {
    fn from(dto: ComponentDto) -> Self {
        BundleComponent { variant_id: dto.variant_id, quantity: dto.quantity }
    }
}

impl From<BundleComponent> for ComponentDto {
    fn from(component: BundleComponent) -> Self {
        ComponentDto { variant_id: component.variant_id, quantity: component.quantity }
    }
}

/// Name and slug of a category in one language.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCategoryText)]
pub(crate) struct CategoryTextDto {
    pub name: String,
    /// Omitted: made from the name. Unique per language in the tenant.
    pub slug: Option<String>,
}

impl From<CategoryTextDto> for CategoryText {
    fn from(dto: CategoryTextDto) -> Self {
        CategoryText { name: dto.name, slug: dto.slug }
    }
}

/// A category of the tenant's tree.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogCategory)]
pub(crate) struct CategoryResponse {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    /// 0 for a top category.
    pub depth: i32,
    /// Display order among siblings.
    pub position: i32,
    /// Shown on the storefront.
    pub visible: bool,
    /// Name and slug per language.
    pub texts: BTreeMap<String, CategoryTextDto>,
    pub updated_at: DateTime<Utc>,
}

impl From<CategoryDetail> for CategoryResponse {
    fn from(detail: CategoryDetail) -> Self {
        CategoryResponse {
            id: detail.category.id,
            parent_id: detail.category.parent_id,
            depth: detail.category.depth,
            position: detail.category.position,
            visible: detail.category.visible,
            texts: detail
                .translations
                .iter()
                .map(|text| {
                    (
                        text.locale.clone(),
                        CategoryTextDto { name: text.name.clone(), slug: Some(text.slug.clone()) },
                    )
                })
                .collect(),
            updated_at: detail.category.updated_at,
        }
    }
}

/// A price list.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPriceList)]
pub(crate) struct PriceListResponse {
    pub id: Uuid,
    /// The store or chain unit the list applies to (and every unit below it).
    pub org_id: Uuid,
    pub name: String,
    #[schema(example = "VND")]
    pub currency: String,
    pub tax_inclusive: bool,
    /// `consumer` or `business`.
    pub audience: String,
    /// Only for this customer group.
    pub customer_group: Option<String>,
    /// Only for this sales channel (step 48).
    pub channel: Option<String>,
    /// Only for this market (step 63).
    pub market: Option<String>,
    /// Only for this B2B company (step 59).
    pub company_id: Option<Uuid>,
    /// Higher wins among lists that tie on unit and filters.
    pub priority: i32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<price_list::Model> for PriceListResponse {
    fn from(row: price_list::Model) -> Self {
        PriceListResponse {
            id: row.id,
            org_id: row.org_id,
            name: row.name,
            currency: row.currency,
            tax_inclusive: row.tax_inclusive,
            audience: row.audience,
            customer_group: row.customer_group,
            channel: row.channel,
            market: row.market,
            company_id: row.company_id,
            priority: row.priority,
            active: row.active,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// A price row. Rows are never changed: a new price is a new row.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPrice)]
pub(crate) struct PriceResponse {
    pub id: Uuid,
    pub price_list_id: Uuid,
    pub variant_id: Uuid,
    pub amount: Money,
    /// Struck-through price, for display only.
    pub compare_at: Option<Money>,
    pub valid_from: DateTime<Utc>,
    /// End of a temporary price; the earlier price applies again after it.
    pub valid_to: Option<DateTime<Utc>>,
    /// `manual`, `derived`, `bulk` or `import`.
    pub source: String,
    pub created_at: DateTime<Utc>,
}

impl From<price::Model> for PriceResponse {
    fn from(row: price::Model) -> Self {
        let currency = row.amount_currency.clone();
        PriceResponse {
            id: row.id,
            price_list_id: row.price_list_id,
            variant_id: row.variant_id,
            amount: Money::from_columns(row.amount_minor, &currency)
                .unwrap_or_else(|_| Money::zero(autoxide_kernel::money::Currency::VND)),
            compare_at: row
                .compare_at_minor
                .and_then(|minor| Money::from_columns(minor, &currency).ok()),
            valid_from: row.valid_from,
            valid_to: row.valid_to,
            source: row.source,
            created_at: row.created_at,
        }
    }
}

/// What to do with a price below the floor: `warn` (saved, with a warning) or `block`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CatalogFloorAction)]
pub(crate) enum FloorActionDto {
    Warn,
    Block,
}

impl From<FloorActionDto> for FloorAction {
    fn from(action: FloorActionDto) -> Self {
        match action {
            FloorActionDto::Warn => FloorAction::Warn,
            FloorActionDto::Block => FloorAction::Block,
        }
    }
}

/// A price floor rule.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = CatalogPriceGuard)]
pub(crate) struct PriceGuardResponse {
    pub id: Uuid,
    pub org_id: Uuid,
    /// Only products in this category (or below); omitted: every product.
    pub category_id: Option<Uuid>,
    pub currency: String,
    pub min_price: Option<Money>,
    /// Minimum margin over unit cost, in basis points (`1500` = 15%).
    pub min_margin_bp: Option<i64>,
    pub action: FloorActionDto,
    pub created_at: DateTime<Utc>,
}

impl From<price_guard::Model> for PriceGuardResponse {
    fn from(row: price_guard::Model) -> Self {
        PriceGuardResponse {
            id: row.id,
            org_id: row.org_id,
            category_id: row.category_id,
            min_price: row
                .min_price_minor
                .and_then(|minor| Money::from_columns(minor, &row.currency).ok()),
            currency: row.currency,
            min_margin_bp: row.min_margin_bp,
            action: if row.action == "block" {
                FloorActionDto::Block
            } else {
                FloorActionDto::Warn
            },
            created_at: row.created_at,
        }
    }
}
