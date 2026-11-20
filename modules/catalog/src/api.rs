//! Hợp đồng của module `catalog` cho module khác (`inventory`, `promotions`, `shipping`, `orders`,
//! `channels`, `marketplace`…): chỉ những gì ở đây được dùng từ bên ngoài
//! (`autoxide_catalog::api::…`). Module khác lấy trait lúc khởi động:
//!
//! ```text
//! let catalog = context.require::<dyn autoxide_catalog::api::Catalog>()?;
//! ```
//!
//! Module đó phải đăng ký SAU `catalog` trong `crates/app/src/modules.rs`. Không module nào đọc
//! bảng `catalog_…` (luật kiến trúc 1). File này không tham chiếu phần bên trong module.

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Số ID tối đa mỗi lần gọi một hàm nhận danh sách ID.
pub const MAX_IDS: usize = 500;

/// Tên event `catalog` phát mà module khác hay nghe (payload `{"before": …, "after": …}`).
pub mod events {
    /// Sản phẩm vừa tạo.
    pub const PRODUCT_CREATED: &str = "catalog.product_created";
    /// Sản phẩm đổi thông tin.
    pub const PRODUCT_UPDATED: &str = "catalog.product_updated";
    /// Sản phẩm đổi trạng thái (`draft`, `active`, `inactive`, `archived`).
    pub const PRODUCT_STATUS_CHANGED: &str = "catalog.product_status_changed";
    /// Sản phẩm được lưu trữ.
    pub const PRODUCT_ARCHIVED: &str = "catalog.product_archived";
    /// Sản phẩm bị xoá (chỉ sản phẩm chưa từng bán).
    pub const PRODUCT_DELETED: &str = "catalog.product_deleted";
    /// Biến thể vừa tạo.
    pub const VARIANT_CREATED: &str = "catalog.variant_created";
    /// Biến thể đổi thông tin (SKU, cân nặng, chế độ bán…).
    pub const VARIANT_UPDATED: &str = "catalog.variant_updated";
    /// Biến thể bị xoá.
    pub const VARIANT_DELETED: &str = "catalog.variant_deleted";
    /// Thêm dòng giá: bảng giá, biến thể, giá mới, `valid_from`.
    pub const PRICE_CHANGED: &str = "catalog.price_changed";
    /// Giá hẹn giờ vừa bắt đầu hay giá tạm thời vừa hết hạn (lịch mỗi phút): `after` có
    /// `variant_ids` và khoảng `from` / `to` đã xét. Giá đang áp của các biến thể đó có thể đã đổi.
    pub const PRICE_EFFECTIVE: &str = "catalog.price_effective";

    /// Event của module `inventory` (bước 42) mà `catalog` nghe để có giá vốn cho sàn giá
    /// (D-222). `catalog` đọc payload `{"after": {"org_id": "<uuid>", "variant_id": "<uuid>",
    /// "unit_cost": {"amount": "<số nguyên đơn vị nhỏ nhất>", "currency": "VND"}}}` (dạng JSON của
    /// `Money`); thiếu trường thì bỏ qua event.
    pub const INVENTORY_COST_CHANGED: &str = "inventory.cost_changed";

    /// Event của module `promotions` (bước 43): giá sau giảm tự động của các biến thể đổi.
    /// `catalog` đọc `after.variant_ids` rồi dựng lại storefront, hỏi giá qua
    /// [`SalePriceSource`](super::SalePriceSource) mà `promotions` cắm vào bằng
    /// [`CatalogExtensions`](super::CatalogExtensions). Thiếu trường thì bỏ qua event.
    pub const PROMOTIONS_AUTOMATIC_PRICES_CHANGED: &str = "promotions.automatic_prices_changed";

    /// Event của module `orders` (bước 45): đơn hoàn tất. `catalog` đọc `after.order_id` và
    /// `after.lines[].{product_id, quantity}` để đếm bán chạy (sắp `best_selling`) và "thường mua
    /// cùng"; nhận lại cùng đơn không đếm hai lần.
    pub const ORDERS_ORDER_COMPLETED: &str = "orders.order_completed";

    /// Event đánh giá của module `orders` (bước 45): đăng, ẩn hay mở lại một đánh giá. `catalog`
    /// đọc `after.product_id` và `after.product_rating = {"count": <số đánh giá đang hiện>, "sum":
    /// <tổng số sao>}` — **tổng hiện tại** của sản phẩm (như `orders_review_stats`), không phải
    /// phần thay đổi, để nhận lại hay nhận lệch thứ tự vẫn đúng. Thiếu trường thì bỏ qua event.
    pub const ORDERS_REVIEW_PUBLISHED: &str = "orders.review_published";
    /// Như [`ORDERS_REVIEW_PUBLISHED`], khi đánh giá bị ẩn hay mở lại.
    pub const ORDERS_REVIEW_MODERATED: &str = "orders.review_moderated";
}

/// Loại sản phẩm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductKind {
    /// Hàng vật lý: cần giao, có tồn kho.
    Physical,
    /// Hàng số: tải về, không giao, không tồn kho.
    Digital,
    /// Dịch vụ: không giao, không tồn kho.
    Service,
    /// Combo / bộ sản phẩm: kho trừ từng thành phần.
    Bundle,
}

/// Trạng thái của sản phẩm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductStatus {
    Draft,
    Active,
    Inactive,
    Archived,
}

/// Chế độ bán của biến thể (D-204).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaleMode {
    /// Chỉ bán khi còn hàng.
    InStock,
    /// Bán cả khi hết hàng (giao khi có).
    Backorder,
    /// Đặt trước hàng chưa về (`preorder_available_on`).
    Preorder,
}

/// Ảnh chụp một biến thể, đủ để chụp vào chứng từ và để module khác quyết định.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VariantSnapshot {
    pub variant_id: Uuid,
    pub product_id: Uuid,
    /// Nút sở hữu sản phẩm (chuỗi, cửa hàng hay người bán).
    pub org_id: OrgId,
    pub kind: ProductKind,
    /// Của sản phẩm; biến thể tắt thì `variant_active = false`.
    pub status: ProductStatus,
    pub variant_active: bool,
    pub sku: String,
    pub barcode: Option<String>,
    /// Tên sản phẩm theo `locale` xin, không có thì ngôn ngữ mặc định của sản phẩm.
    pub name: String,
    /// Nhãn tổ hợp tuỳ chọn, vd "Đỏ / M"; rỗng khi sản phẩm không có tuỳ chọn.
    pub variant_label: String,
    /// Ngôn ngữ thật của `name` và `variant_label`.
    pub locale: String,
    /// Mã nhóm thuế của engine thuế (bước 36), vd `vat.standard`.
    pub tax_category: String,
    /// Mọi danh mục của sản phẩm KÈM tổ tiên của chúng.
    pub category_ids: Vec<Uuid>,
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    /// `false` với hàng số và dịch vụ.
    pub requires_shipping: bool,
    /// `false` với hàng số, dịch vụ và combo (combo trừ kho từng thành phần).
    pub track_inventory: bool,
    pub sale_mode: SaleMode,
    /// Ngày dự kiến có hàng khi `Preorder` (theo múi giờ của nút).
    pub preorder_available_on: Option<NaiveDate>,
    /// Tổng suất đặt trước; `None` = không giới hạn.
    pub preorder_limit: Option<i64>,
    /// Hết nhận đặt trước.
    pub preorder_ends_at: Option<DateTime<Utc>>,
    /// Cờ cũ, còn đọc được tới khi `inventory`, `orders` chuyển hết sang `sale_mode` (D-204):
    /// `true` khi `sale_mode` là `Backorder`.
    pub allow_backorder: bool,
    pub lot_tracked: bool,
    pub serial_tracked: bool,
    /// Số tháng bảo hành; `None` = không bảo hành.
    pub warranty_months: Option<i32>,
    pub image_media_id: Option<Uuid>,
    /// Phiên bản của biến thể, để bên chụp ghi kèm.
    pub updated_at: DateTime<Utc>,
}

/// Câu hỏi giá: giá của các biến thể ở cửa hàng `org_id` lúc `at`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PriceQuery {
    /// Cửa hàng bán.
    pub org_id: OrgId,
    /// Công ty B2B (bước 59; giờ `None`) — D-222.
    pub company_id: Option<Uuid>,
    /// Mã nhóm khách (bước 47 cung cấp; giờ là chữ tự do).
    pub customer_group: Option<String>,
    /// Kênh bán (bước 48; giờ `None`) — D-222.
    pub channel: Option<String>,
    /// Thị trường (bước 63; giờ `None`) — D-222.
    pub market: Option<String>,
    pub currency: Currency,
    pub at: DateTime<Utc>,
    /// Tối đa [`MAX_IDS`].
    pub variant_ids: Vec<Uuid>,
}

impl PriceQuery {
    /// Câu hỏi giá chung (không nhóm khách, kênh, thị trường) ở cửa hàng `org_id` lúc `at`.
    pub fn new(
        org_id: OrgId,
        currency: Currency,
        at: DateTime<Utc>,
        variant_ids: Vec<Uuid>,
    ) -> Self {
        PriceQuery {
            org_id,
            company_id: None,
            customer_group: None,
            channel: None,
            market: None,
            currency,
            at,
            variant_ids,
        }
    }

    /// Hỏi giá của nhóm khách `group`.
    pub fn for_customer_group(mut self, group: impl Into<String>) -> Self {
        self.customer_group = Some(group.into());
        self
    }

    /// Hỏi giá của kênh bán `channel` (bước 48).
    pub fn for_channel(mut self, channel: impl Into<String>) -> Self {
        self.channel = Some(channel.into());
        self
    }

    /// Hỏi giá của thị trường `market` (bước 63).
    pub fn for_market(mut self, market: impl Into<String>) -> Self {
        self.market = Some(market.into());
        self
    }

    /// Hỏi giá của công ty B2B `company_id` (bước 59).
    pub fn for_company(mut self, company_id: Uuid) -> Self {
        self.company_id = Some(company_id);
        self
    }
}

/// Giá của một biến thể, đủ để chụp vào chứng từ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PriceSnapshot {
    pub variant_id: Uuid,
    /// Bảng giá thắng theo thứ tự chọn bảng giá (D-222).
    pub price_list_id: Uuid,
    /// Dòng giá cụ thể — chụp vào chứng từ.
    pub price_id: Uuid,
    /// Giá một đơn vị.
    pub amount: Money,
    /// Giá đã gồm thuế.
    pub tax_inclusive: bool,
    /// Giá gạch (chỉ để hiển thị).
    pub compare_at: Option<Money>,
    pub valid_from: DateTime<Utc>,
    /// Hết hiệu lực (giá tạm thời); `None` = tới khi có dòng mới.
    pub valid_to: Option<DateTime<Utc>>,
}

/// Khi giá dưới sàn thì làm gì (D-222).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FloorAction {
    /// Ghi được, kèm cảnh báo.
    Warn,
    /// Không ghi được.
    Block,
}

/// Sàn giá đang áp cho một biến thể ở một nút (D-222): quy tắc của nút gần nhất, danh mục gần
/// nhất thắng. `promotions` kiểm lúc kích hoạt và cắt giá khi tính.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PriceFloor {
    pub variant_id: Uuid,
    /// Quy tắc sàn giá thắng.
    pub guard_id: Uuid,
    /// Giá tối thiểu tuyệt đối, nếu quy tắc có.
    pub min_price: Option<Money>,
    /// Biên lợi nhuận tối thiểu trên giá vốn, theo phần vạn (`1500` = 15%), nếu quy tắc có.
    pub min_margin_bp: Option<i64>,
    /// Giá vốn đơn vị đã biết (từ event `inventory.cost_changed`), nếu có.
    pub unit_cost: Option<Money>,
    /// Giá thấp nhất được phép: lớn hơn của `min_price` và giá vốn × (1 + biên); `None` khi
    /// quy tắc chỉ có biên mà chưa biết giá vốn.
    pub floor: Option<Money>,
    pub action: FloorAction,
}

/// Một thành phần của combo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleComponent {
    pub variant_id: Uuid,
    /// Số đơn vị của thành phần trong một combo; > 0.
    pub quantity: i64,
}

/// Đọc sản phẩm, biến thể và giá cho module khác.
//
// `#[async_trait]`: Rust chưa cho gọi hàm `async` của trait qua `dyn Trait`; macro này đổi hàm
// `async` thành hàm trả về `Box<dyn Future>` để gọi được.
#[async_trait::async_trait]
pub trait Catalog: Send + Sync {
    /// Biến thể theo ID, theo thứ tự `ids`, bỏ ID không có. Quá [`MAX_IDS`] thì lỗi
    /// `catalog.too_many_ids`. Một lần gọi là vài câu query, không phụ thuộc số ID.
    async fn variants(
        &self,
        tenant: &TenantContext,
        ids: &[Uuid],
        locale: Option<&str>,
    ) -> Result<Vec<VariantSnapshot>, Error>;

    /// Một biến thể theo SKU (không phân biệt hoa thường, bỏ khoảng trắng) hoặc mã vạch — bán tại
    /// quầy, đa kênh.
    async fn variant_by_code(
        &self,
        tenant: &TenantContext,
        code: &str,
        locale: Option<&str>,
    ) -> Result<Option<VariantSnapshot>, Error>;

    /// Giá đang hiệu lực lúc `query.at` (dòng có `valid_from <= at`, chưa hết hạn), theo thứ tự
    /// chọn bảng giá (D-222). Biến thể không có giá thì vắng trong kết quả.
    async fn prices(
        &self,
        tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error>;

    /// Giá hiệu lực NGAY TRƯỚC `query.at` (dòng có `valid_from < at`) — "giá ngay trước khuyến
    /// mại" (bước 43). Cùng bảng giá như [`prices`](Self::prices).
    async fn reference_prices(
        &self,
        tenant: &TenantContext,
        query: &PriceQuery,
    ) -> Result<Vec<PriceSnapshot>, Error>;

    /// Sàn giá đang áp cho các biến thể ở nút `org_id`, tiền `currency` (D-222). Biến thể không
    /// có quy tắc nào thì vắng.
    async fn price_floors(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        currency: Currency,
        variant_ids: &[Uuid],
    ) -> Result<Vec<PriceFloor>, Error>;

    /// Thành phần của các biến thể combo (biến thể không phải combo thì vắng).
    async fn bundle_components(
        &self,
        tenant: &TenantContext,
        variant_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error>;
}

/// Giá sau giảm tự động của một biến thể (từ khuyến mại không cần mã, không điều kiện giỏ).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SalePrice {
    pub variant_id: Uuid,
    /// Giá một đơn vị sau giảm, cùng tiền tệ với giá niêm yết đã gửi.
    pub sale_price: Money,
    /// Lúc giá giảm hết.
    pub ends_at: DateTime<Utc>,
}

impl SalePrice {
    pub fn new(variant_id: Uuid, sale_price: Money, ends_at: DateTime<Utc>) -> Self {
        SalePrice { variant_id, sale_price, ends_at }
    }
}

/// Nguồn giá sau giảm tự động cho storefront công khai. `promotions` đăng ký SAU `catalog` và phụ
/// thuộc vào `catalog`, nên `catalog` không gọi thẳng trait của nó được (vòng phụ thuộc): module
/// khuyến mại cài trait này và cắm vào bằng [`CatalogExtensions::set_sale_price_source`].
#[async_trait::async_trait]
pub trait SalePriceSource: Send + Sync {
    /// Giá sau giảm của các biến thể ở nút bán `org_id` lúc `at`, từ giá niêm yết `prices`
    /// (biến thể, giá). Biến thể không được giảm thì vắng. Gọi ngoài transaction.
    async fn sale_prices(
        &self,
        tenant: &TenantContext,
        org_id: OrgId,
        at: DateTime<Utc>,
        prices: &[(Uuid, Money)],
    ) -> Result<Vec<SalePrice>, Error>;
}

/// Sản phẩm sắp mở bán, đưa cho [`ListingGate`] xét.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListingCandidate {
    pub product_id: Uuid,
    /// Nút sở hữu sản phẩm (người bán khi là sàn).
    pub org_id: OrgId,
    /// Danh mục của sản phẩm (không kèm tổ tiên).
    pub category_ids: Vec<Uuid>,
    pub kind: ProductKind,
}

impl ListingCandidate {
    pub fn new(
        product_id: Uuid,
        org_id: OrgId,
        category_ids: Vec<Uuid>,
        kind: ProductKind,
    ) -> Self {
        ListingCandidate { product_id, org_id, category_ids, kind }
    }
}

/// Kết quả xét mở bán.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListingDecision {
    Allow,
    /// Không cho mở bán. `code` là mã lỗi ổn định của module cắm vào (vd
    /// `marketplace.seller_not_verified`, có bản dịch ở module đó), `message` nói rõ phải làm gì.
    Deny {
        code: String,
        message: String,
    },
}

/// Điều kiện mở bán do module khác đặt (bước 46: người bán đã xác thực, sản phẩm đã duyệt, đủ giấy
/// tờ ngành hàng). `catalog` hỏi trước khi chuyển sản phẩm sang `active`, ngoài transaction.
#[async_trait::async_trait]
pub trait ListingGate: Send + Sync {
    async fn can_publish(
        &self,
        tenant: &TenantContext,
        candidate: &ListingCandidate,
    ) -> Result<ListingDecision, Error>;
}

/// Điểm cắm của module đăng ký sau `catalog`: lấy bằng
/// `context.require::<dyn CatalogExtensions>()` trong `Module::start`.
pub trait CatalogExtensions: Send + Sync {
    /// Cắm nguồn giá sau giảm tự động (một nguồn; cắm lần hai thì giữ nguồn đầu và trả `false`).
    fn set_sale_price_source(&self, source: std::sync::Arc<dyn SalePriceSource>) -> bool;

    /// Cắm điều kiện mở bán (một điều kiện; cắm lần hai thì giữ cái đầu và trả `false`). Có cài
    /// đặt mặc định (không nhận, trả `false`) để bản cài đặt sẵn có — vd bản giả trong test của
    /// module khác — không phải sửa khi trait có thêm hàm.
    fn set_listing_gate(&self, gate: std::sync::Arc<dyn ListingGate>) -> bool {
        let _ = gate;
        false
    }
}
