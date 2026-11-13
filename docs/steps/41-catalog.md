# Bước 41 — Sản phẩm (`catalog`)

> Lộ trình: `docs/ROADMAP.md` bước 41 và phần "Giai đoạn module thương mại" · Thiết kế:
> `docs/research/business-foundation.md` mục 8, 9 · Quyết định: D-180 (`Money`), D-184
> (`reference`), D-185, D-186, D-188 (thuế), D-191, D-194

## Mục tiêu

Xong bước này, mọi khách (cửa hàng độc lập, chuỗi, SaaS, sàn) có một danh mục sản phẩm dùng chung
cho website, bán tại quầy và các module thương mại sau: sản phẩm có biến thể, danh mục dạng cây với
thuộc tính bắt buộc là dữ liệu, bảng giá theo cửa hàng / nhóm khách có lịch sử, đọc công khai cho
website (lọc, tìm, ETag, sitemap), nhập / xuất CSV, hàng số / dịch vụ / combo, sản phẩm tương tự.
`inventory`, `promotions`, `orders`, `channels` đọc sản phẩm và giá qua `autoxide_catalog::api`,
không đọc bảng `catalog_…`.

## Bắt đầu khi

- Bước 35 xong (`Module::features`, `Module::limits`, `dyn Entitlements` của kernel) và bước 36 xong
  (`dyn TaxEngine` của `autoxide_tax::api` để kiểm mã nhóm thuế; `rust_decimal`).
- Có sẵn từ bước 33–34: `TenantId`, `OrgId`, `TenantContext`, `Scope`, công cụ lọc theo khách và theo
  nhánh, `dyn OrgDirectory` (`autoxide_organizations::api`), cài đặt kế thừa theo cây (ngôn ngữ, tiền
  tệ), token / URL ký mang khách (bước 33), dấu `ai_assisted` (bước 37).
- Module sẵn có: `media` (`dyn Media`), `files` (`dyn Storage`), `ai` (`dyn TextModel`, `dyn Embedder`).

## Phạm vi

- Crate mới `modules/catalog` (`autoxide-catalog`), đăng ký trong `crates/app/src/modules.rs` sau
  `notifications`, trước `inventory`.
- **Commit chuẩn bị chung cho 42–44** (làm ở đây để ba bước song song sau không đụng nhau ở kernel):
  1. `autoxide_kernel::csv` — dời `modules/forms/src/csv.rs` (ghi RFC 4180, chặn CSV injection, BOM,
     CRLF) vào kernel, thêm bộ đọc; `forms` gọi bản của kernel. **API mới cần thêm**:
     `pub fn write(header: &[String], rows: &[Vec<String>]) -> String`;
     `pub fn read(text: &str, max_rows: usize) -> Result<Vec<Vec<String>>, CsvError>` (bỏ BOM, nhận
     CRLF / LF, ngoặc kép, xuống dòng trong ô; lỗi nêu số dòng). Đọc tự viết hay crate `csv` — xem
     "Agent tự quyết".
  2. `autoxide_kernel::slug` — dời `modules/content/src/entries/slug.rs` (`is_valid`, `from_title`,
     `with_suffix`, `MAX_LENGTH`) vào kernel thành `pub`; `content` gọi bản của kernel. Hai module một
     quy tắc slug.
  3. `Subscription::optional(event, handler)` trong `autoxide_kernel::event` — **API mới cần thêm**:
     như `Subscription::new` nhưng registry không báo lỗi khởi động khi chưa module nào khai event đó
     (sửa `ModuleRegistry::dispatcher`, ghi log `info`). `inventory` (42) và `shipping` (44) nhận event
     của `orders` (45) — module chưa tồn tại khi hai bước đó làm. Ghi quyết định.
- Ngoài phạm vi: tồn kho (42), khuyến mại (43), cân nặng tính phí (44 dùng số liệu của bước này), giỏ
  hàng và đơn (45), duyệt sản phẩm của người bán trên sàn (46 — bước này chỉ có cổng chặn hàng cấm /
  hàng có điều kiện), nhóm khách thật (47 — ở đây nhóm khách là mã chữ tự do), giá theo kênh (48).

## Thiết kế

### Kiểu và trait

Phác thảo, tra API thật bằng skill `check-library-api`; tên kiểu của bước 33–36 theo đặc tả của các
bước đó.

```rust
// modules/catalog/src/api.rs — hợp đồng cho inventory, promotions, orders, channels, marketplace
pub const MAX_IDS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProductKind { Physical, Digital, Service, Bundle }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProductStatus { Draft, Active, Inactive, Archived }

/// Ảnh chụp một biến thể, đủ để chụp vào chứng từ và để module khác quyết định.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VariantSnapshot {
    pub variant_id: Uuid,
    pub product_id: Uuid,
    pub org_id: OrgId,                 // nút sở hữu sản phẩm (chuỗi, cửa hàng hay người bán)
    pub kind: ProductKind,
    pub status: ProductStatus,          // của sản phẩm; biến thể tắt thì `variant_active = false`
    pub variant_active: bool,
    pub sku: String,
    pub barcode: Option<String>,
    pub name: String,                   // theo `locale` xin, không có thì ngôn ngữ mặc định
    pub variant_label: String,          // "Đỏ / M"
    pub locale: String,                 // ngôn ngữ thật của `name`
    pub tax_category: String,           // mã nhóm thuế của engine thuế (bước 36)
    pub category_ids: Vec<Uuid>,        // mọi danh mục của sản phẩm KÈM tổ tiên
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>, pub width_mm: Option<i64>, pub height_mm: Option<i64>,
    pub requires_shipping: bool,        // false với Digital, Service
    pub track_inventory: bool,          // false với Digital, Service, Bundle (combo trừ thành phần)
    pub allow_backorder: bool,
    pub lot_tracked: bool,
    pub serial_tracked: bool,
    pub image_media_id: Option<Uuid>,
    pub updated_at: DateTime<Utc>,      // phiên bản, để bên chụp ghi kèm
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PriceQuery {
    pub org_id: OrgId,                  // cửa hàng bán
    pub customer_group: Option<String>, // mã nhóm khách (bước 47 cung cấp; giờ là chữ tự do)
    pub currency: Currency,
    pub at: DateTime<Utc>,
    pub variant_ids: Vec<Uuid>,         // tối đa MAX_IDS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PriceSnapshot {
    pub variant_id: Uuid,
    pub price_list_id: Uuid,
    pub price_id: Uuid,                 // dòng giá cụ thể — chụp vào chứng từ
    pub amount: Money,                  // giá một đơn vị
    pub tax_inclusive: bool,
    pub compare_at: Option<Money>,      // giá gạch (chỉ để hiển thị)
    pub valid_from: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleComponent { pub variant_id: Uuid, pub quantity: i64 }

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadRequest {
    pub order_id: Uuid,
    pub order_line_id: Uuid,            // khoá chống trùng: gọi lại không phát thêm
    pub buyer_id: Option<Uuid>,
    pub variant_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DownloadLink {
    pub asset_id: Uuid, pub file_name: String,
    pub url: String,                    // trang tải công khai mang token (chỉ trả lần phát đầu)
    pub remaining: i64, pub expires_at: DateTime<Utc>,
}

#[async_trait::async_trait]
pub trait Catalog: Send + Sync {
    /// Biến thể theo ID, theo thứ tự `ids`, bỏ ID không có. Quá MAX_IDS thì lỗi.
    async fn variants(&self, tenant: &TenantContext, ids: &[Uuid], locale: Option<&str>)
        -> Result<Vec<VariantSnapshot>, Error>;
    /// Tìm một biến thể theo SKU hoặc mã vạch (bán tại quầy, đa kênh).
    async fn variant_by_code(&self, tenant: &TenantContext, code: &str, locale: Option<&str>)
        -> Result<Option<VariantSnapshot>, Error>;
    /// Giá đang hiệu lực lúc `query.at`. Biến thể không có giá thì vắng trong kết quả.
    async fn prices(&self, tenant: &TenantContext, query: &PriceQuery)
        -> Result<Vec<PriceSnapshot>, Error>;
    /// Giá hiệu lực NGAY TRƯỚC `query.at` (thời điểm < at) — "giá ngay trước khuyến mại" (bước 43).
    async fn reference_prices(&self, tenant: &TenantContext, query: &PriceQuery)
        -> Result<Vec<PriceSnapshot>, Error>;
    /// Thành phần của các biến thể combo (biến thể không phải combo thì vắng).
    async fn bundle_components(&self, tenant: &TenantContext, variant_ids: &[Uuid])
        -> Result<HashMap<Uuid, Vec<BundleComponent>>, Error>;
    /// Phát quyền tải hàng số cho một dòng đơn đã thanh toán. Gọi lại cùng `order_line_id`: trả
    /// quyền đã phát (không kèm `url` — token không lưu dạng đọc được).
    async fn issue_downloads(&self, tenant: &TenantContext, request: &DownloadRequest)
        -> Result<Vec<DownloadLink>, Error>;
    /// Thu hồi mọi quyền tải của đơn (hoàn tiền, huỷ). Trả số quyền vừa thu hồi; gọi lại trả 0.
    async fn revoke_downloads(&self, tenant: &TenantContext, order_id: Uuid) -> Result<u64, Error>;
}
```

`tenant: &TenantContext` ở mọi hàm: bên cài lấy database của khách qua nó (bước 33). Nếu bước 33 chọn
cách khác để chuyển ngữ cảnh khách qua trait, đổi đồng loạt ở cả bốn `api` của bước 41–44.

Quyền theo độ sâu: handler gọi công cụ của bước 34 (phác thảo: `principal.require_in(&PERM, org_id)`
cho bản ghi có `org_id`, và bộ lọc nhánh cho danh sách). Danh mục, thuộc tính, bảng mã hàng có điều
kiện thuộc cả khách: cần quyền ở độ sâu `Tenant`.

### Bảng dữ liệu

Mọi bảng cấp khách có `tenant_id` (`uuid`), unique và index bắt đầu bằng `tenant_id`. Kiểu: `uuid`,
`string_len`, `text`, `big_integer` (tiền `*_minor` + `*_currency` `string_len(3)`, số lượng, mm, gram),
`integer`, `boolean`, `timestamp_with_time_zone` (UTC), `date` (ngày hiệu lực, hạn giấy tờ). JSON lưu
trong cột `text` (như `audit`). Không dùng `jsonb`, mảng, enum của Postgres. Migration `m0001…` (bảng
chính) và `m0002…` (bảng phụ), mỗi cái có `down`; thêm `outbox_migration`, `jobs_migration`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `catalog_products` | `id`, `tenant_id`, `org_id`, `kind`, `status`, `tax_category`, `primary_category_id?`, `track_inventory`, `allow_backorder`, `lot_tracked`, `serial_tracked`, `default_locale`, `ai_assisted`, `created_by`, `created_at`, `updated_at`, `activated_at?`, `ever_active` | (`tenant_id`,`org_id`,`status`,`id`); (`tenant_id`,`updated_at`,`id`) |
| `catalog_product_translations` | `id`, `tenant_id`, `product_id`, `locale`, `name`, `slug`, `short_description`, `description`, `meta_title`, `meta_description`, `search_key` (= `search::fold` của tên + SKU + mô tả ngắn), `name_key` (= `search::normalize`, để sắp) | unique (`tenant_id`,`locale`,`slug`); unique (`tenant_id`,`product_id`,`locale`) |
| `catalog_options` / `catalog_option_values` | `product_id`, `position`, `names` (JSON `{locale: tên}`) / `option_id`, `position`, `labels` (JSON) | (`tenant_id`,`product_id`) |
| `catalog_variants` | `id`, `tenant_id`, `product_id`, `sku`, `sku_key` (chữ hoa, bỏ khoảng trắng), `barcode?`, `option_key` (ID giá trị tuỳ chọn sắp tăng, nối `,`), `position`, `active`, `weight_grams?`, `length_mm?`, `width_mm?`, `height_mm?`, `image_media_id?`, `updated_at` | unique (`tenant_id`,`sku_key`); unique (`tenant_id`,`barcode`) (NULL lặp được trên cả hai DB); unique (`tenant_id`,`product_id`,`option_key`) |
| `catalog_product_media` | `product_id`, `media_id`, `variant_id?`, `position`, `alt` | (`tenant_id`,`product_id`,`position`) |
| `catalog_categories` | `id`, `tenant_id`, `parent_id?`, `depth`, `position`, `updated_at` | (`tenant_id`,`parent_id`,`position`) |
| `catalog_category_paths` | `ancestor_id`, `descendant_id`, `depth` (bảng tổ tiên – con cháu, gồm chính nó) | unique (`tenant_id`,`ancestor_id`,`descendant_id`); (`tenant_id`,`descendant_id`) |
| `catalog_category_translations` | `category_id`, `locale`, `name`, `slug` | unique (`tenant_id`,`locale`,`slug`) |
| `catalog_product_categories` | `product_id`, `category_id` | unique (`tenant_id`,`product_id`,`category_id`); (`tenant_id`,`category_id`,`product_id`) |
| `catalog_attributes` | `id`, `tenant_id`, `code`, `data_type` (`text`/`integer`/`decimal`/`boolean`/`date`/`option`/`options`), `scale` (số chữ số thập phân khi `decimal`), `unit?`, `names` (JSON), `filterable` | unique (`tenant_id`,`code`) |
| `catalog_attribute_options` | `attribute_id`, `code`, `labels` (JSON), `position` | unique (`tenant_id`,`attribute_id`,`code`) |
| `catalog_category_attributes` | `category_id`, `attribute_id`, `required`, `applies_to_descendants`, `effective_from` (`date`), `effective_to?`, `legal_basis?` | (`tenant_id`,`category_id`) |
| `catalog_product_attributes` | `product_id`, `attribute_id`, `value_text?`, `value_integer?` (decimal lưu nguyên theo `scale`), `value_boolean?`, `value_date?`, `option_id?` | (`tenant_id`,`attribute_id`,`option_id`,`product_id`); (`tenant_id`,`attribute_id`,`value_integer`) |
| `catalog_restriction_rules` | **dữ liệu dùng chung mọi khách (mặt điều khiển, như `address`)**: `id`, `jurisdiction` (`VN`), `code`, `level` (`prohibited`/`conditional`), `names` (JSON), `required_documents` (JSON: mã loại giấy tờ), `legal_basis`, `source_url`, `effective_from`, `effective_to?`, `pack_version` | unique (`jurisdiction`,`code`,`effective_from`) |
| `catalog_category_restrictions` | `category_id`, `rule_code` | unique (`tenant_id`,`category_id`,`rule_code`) |
| `catalog_product_documents` | `id`, `product_id`, `org_id`, `document_type`, `number`, `issued_by`, `issued_on`, `expires_on?`, `file_key?` (trong `Storage`), `created_at` | (`tenant_id`,`product_id`) |
| `catalog_bundle_components` | `bundle_variant_id`, `component_variant_id`, `quantity` | unique (`tenant_id`,`bundle_variant_id`,`component_variant_id`) |
| `catalog_price_lists` | `id`, `tenant_id`, `org_id`, `name`, `currency`, `tax_inclusive`, `audience` (`consumer`/`business`), `customer_group?`, `priority`, `active`, `updated_at` | (`tenant_id`,`org_id`,`active`) |
| `catalog_prices` | **chỉ thêm**: `id`, `tenant_id`, `price_list_id`, `variant_id`, `amount_minor`, `amount_currency`, `compare_at_minor?`, `valid_from`, `valid_to?`, `reference?`, `reference_fingerprint?`, `created_by`, `created_at` | (`tenant_id`,`price_list_id`,`variant_id`,`valid_from`); unique (`tenant_id`,`price_list_id`,`reference`) |
| `catalog_storefront_entries` | bảng dẫn xuất cho đọc công khai: `tenant_id`, `org_id` (cửa hàng), `product_id`, `visible`, `min_price_minor?`, `max_price_minor?`, `currency`, `published_at`, `refreshed_at` | unique (`tenant_id`,`org_id`,`product_id`); (`tenant_id`,`org_id`,`visible`,`min_price_minor`,`product_id`); (`tenant_id`,`org_id`,`visible`,`published_at`,`product_id`) |
| `catalog_store_overrides` | `org_id`, `product_id`, `hidden` (cửa hàng ẩn sản phẩm của chuỗi) | unique (`tenant_id`,`org_id`,`product_id`) |
| `catalog_digital_assets` | `id`, `variant_id`, `file_name`, `content_type`, `size`, `sha256`, `storage_key` (`catalog/digital/<id>`), `download_limit`, `grant_days` | (`tenant_id`,`variant_id`) |
| `catalog_download_grants` | `id`, `tenant_id`, `order_id`, `order_line_id`, `buyer_id?`, `asset_id`, `token_hash` (SHA-256), `remaining`, `expires_at`, `revoked_at?`, `created_at` | unique (`tenant_id`,`order_line_id`,`asset_id`); unique (`token_hash`); (`tenant_id`,`order_id`) |
| `catalog_transfers` | việc nhập / xuất: `id`, `tenant_id`, `kind` (`import`/`export`), `status`, `file_key`, `total_rows`, `ok_rows`, `errors` (JSON, tối đa 1.000 lỗi), `created_by`, `created_at`, `finished_at?` | (`tenant_id`,`created_at`,`id`) |
| `catalog_suggestions` | bản nháp AI: `id`, `product_id`, `locale`, `fields` (JSON), `status` (`pending`/`applied`/`discarded`), `created_by`, `created_at` | (`tenant_id`,`product_id`) |
| `catalog_embeddings` | như `content_embeddings`: `product_id`, `locale`, `chunk_index`, `content_hash`, `model`, `dimensions`, `vector` (`binary`) | unique (`tenant_id`,`product_id`,`locale`,`chunk_index`) |

Giá: dòng giá **không bao giờ sửa hay xoá**. Đổi giá = thêm dòng với `valid_from` mới; giá hiệu lực
lúc `t` = dòng có `valid_from` lớn nhất ≤ `t` và (`valid_to` NULL hay > `t`). Lịch sử giá = các dòng.
Chọn bảng giá: các bảng `active`, cùng `currency`, `org_id` là cửa hàng hay tổ tiên của nó (hỏi
`OrgDirectory`), `customer_group` khớp hay NULL; thứ tự ưu tiên: nút gần cửa hàng nhất (cửa hàng ghi
đè chuỗi — khuôn mẫu kế thừa của bước 34) → có nhóm khách hơn không nhóm → `priority` → `id`.

### API REST

Dưới `/api/v1/catalog`. Mọi handler: một dòng `///`, `description` cho từng response kèm mã lỗi,
nêu permission. Thân JSON bằng `ValidJson<T>`; tạo mới trả `Created`; bản ghi sửa được trả `Tagged`
(ETag) và nhận `If-Match`. Danh sách dùng `fetch_page` / `fetch_page_by`.

Quản trị:

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /products` · `POST /products` | `catalog.products.read` · `catalog.products.write` | lọc `status`, `kind`, `category_id`, `org_id`, `q`; tạo giữ hạn mức `catalog.products` |
| `GET/PATCH/DELETE /products/{id}` | read · write | xoá chỉ khi `ever_active = false`, còn lại thì lưu trữ |
| `POST /products/{id}/status` | `catalog.products.publish` | `{status}`; kiểm chuyển trạng thái, thuộc tính bắt buộc, giấy tờ, file số, nhóm thuế |
| `PUT/DELETE /products/{id}/translations/{locale}` | write | slug tự sinh từ tên khi trống (`autoxide_kernel::slug`) |
| `PUT /products/{id}/options` · `PUT /products/{id}/attributes` · `PUT /products/{id}/media` · `PUT /products/{id}/categories` | write | thay cả tập; ảnh phải là `media` `Public` |
| `POST /products/{id}/variants` · `PATCH/DELETE /variants/{id}` · `GET /variants?code=` | write · read | tính năng `catalog.variants` khi > 1 biến thể |
| `PUT /variants/{id}/components` | write | tính năng `catalog.bundles` |
| `POST/DELETE /variants/{id}/digital-assets[/{asset}]` | write | tải file lên (khai `Module::body_limits`), tính năng `catalog.digital` |
| `GET/POST/DELETE /products/{id}/documents[/{doc}]` | `catalog.documents.manage` | giấy tờ hàng có điều kiện |
| `GET/POST /categories` · `PATCH/DELETE /categories/{id}` · `POST /categories/{id}/move` · `PUT /categories/{id}/translations/{locale}` · `PUT /categories/{id}/attributes` · `PUT /categories/{id}/restrictions` | `catalog.categories.manage` (độ sâu `Tenant`); đọc bằng `catalog.products.read` | dời khoá dòng, kiểm vòng lặp |
| `GET/POST /attributes` · `PATCH /attributes/{id}` · `PUT /attributes/{id}/options` | `catalog.categories.manage` | |
| `GET /restriction-rules` | `catalog.products.read` | dữ liệu dùng chung, chỉ đọc |
| `GET/POST /price-lists` · `PATCH /price-lists/{id}` | `catalog.prices.read` · `catalog.prices.write` | tính năng `catalog.price_lists` khi > 1 bảng giá |
| `POST /price-lists/{id}/prices` | `catalog.prices.write` | thêm tối đa 500 dòng giá; `reference` bắt buộc (`submit_once`) |
| `GET /price-lists/{id}/prices?variant_id=` | `catalog.prices.read` | lịch sử, mới trước |
| `GET /variants/{id}/price?org_id=&customer_group=&currency=&at=` | `catalog.prices.read` | giá giải ra và vì sao (bảng nào thắng) — cho agent kiểm |
| `PUT /stores/{org_id}/overrides/{product_id}` | `catalog.products.write` ở nút cửa hàng | ẩn / hiện |
| `POST /imports` · `GET /imports/{id}` | `catalog.import` | CSV (multipart) → 202 + vị trí theo dõi; lỗi theo dòng |
| `POST /exports` · `GET /exports/{id}` · `GET /exports/{id}/content` | `catalog.export` | |
| `POST /products/{id}/suggestions` · `POST /suggestions/{id}/apply` · `POST /suggestions/{id}/discard` | `catalog.products.write` | AI: mô tả, SEO; áp vào bản dịch và bật `ai_assisted` |
| `GET /download-grants?order_id=` · `POST /download-grants/{id}/revoke` | `catalog.downloads.manage` | |

Công khai (`security((), ("bearer" = []))`, `Cache-Control: public, no-cache`, ETag là dấu vân tay của cả
câu trả lời như `content::http::public::fingerprint`, 304 khi khớp). Cửa hàng lấy từ `TenantContext`
(tên miền → nút, bước 33–34); tên miền thuộc cả khách mà khách có nhiều cửa hàng thì bắt buộc
`store_id`:

- `GET /public/products?locale&category=<slug>&attr.<code>=<giá trị>&price_min&price_max&q&sort=newest|price_asc|price_desc` + `PageQuery`
  — chỉ `active`, thấy ở cửa hàng; giá và sắp theo giá đọc `catalog_storefront_entries`
  (`fetch_page_by` trên `min_price_minor`); `q` tối đa 200 ký tự, `search::contains`.
- `GET /public/products/by-slug/{slug}?locale` — sản phẩm, biến thể, giá hiện tại (không nhóm khách),
  ảnh (`Media::find_many`), thuộc tính, bản dịch khác (đường dẫn), thông tin nhãn bắt buộc.
- `GET /public/categories?locale` (cây) · `GET /public/categories/{slug}?locale`
- `GET /public/sitemap?locale` (+ `PageQuery`, tối đa 200 / trang) — đường dẫn, bản dịch, `modified_at`.
- `GET /public/products/{id}/similar?locale&limit` — theo embedding; embedding tắt thì cùng danh mục.
- `POST /public/downloads/{token}` — trừ một lượt (`rows::adjust` trên `remaining`, `NON_NEGATIVE`),
  trả URL ký sống `AUTOXIDE_CATALOG_DOWNLOAD_LINK_SECONDS`; `GET /public/downloads/{token}/file?exp&sig`
  — phục vụ file qua `Storage::get_range` (nhận `Range`), không trừ lượt. URL ký mang khách (bước 33).

Đường dẫn công khai do máy chủ tính: `<AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX>/<slug>`, không có tiền
tố ngôn ngữ (như `content`).

### Tool MCP

Cùng permission và service với REST: `catalog.list_products`, `catalog.get_product`,
`catalog.create_product`, `catalog.update_product`, `catalog.set_product_status`,
`catalog.upsert_variant`, `catalog.list_categories`, `catalog.add_prices`, `catalog.resolve_price`
(`read_only`), `catalog.start_import`, `catalog.import_status`, `catalog.suggest_copy`.
`delete_product` đánh dấu `.destructive()`.

### Event

Payload `{"before": …, "after": …}`, không có token tải.

`catalog.product_created` · `catalog.product_updated` · `catalog.product_status_changed` ·
`catalog.product_deleted` · `catalog.variant_created` · `catalog.variant_updated` ·
`catalog.variant_deleted` · `catalog.price_changed` (lúc thêm dòng giá: bảng giá, biến thể, giá cũ /
mới, `valid_from`) · `catalog.price_effective` (lúc giá hẹn giờ bắt đầu hay hết hiệu lực — cho
`channels`) · `catalog.category_changed` · `catalog.import_completed` · `catalog.export_completed` ·
`catalog.downloads_issued` · `catalog.downloads_revoked`.

Nhận: `organizations.node_moved` / `organizations.node_status_changed` (tên theo đặc tả bước 34) →
xếp job dựng lại `catalog_storefront_entries` của nhánh. `privacy::USER_DELETED` → bỏ `created_by`.

### Job và lịch

- `catalog.import` (một dòng một transaction ngắn, gộp 100 dòng; dòng lỗi ghi vào `errors`, không dừng
  cả file), `catalog.export`, `catalog.refresh_storefront` (theo sản phẩm hay theo cửa hàng; xếp
  trong cùng transaction với thay đổi giá / trạng thái / ẩn hiện), `catalog.index_embeddings`.
- Lịch: `catalog.apply_scheduled_prices` mỗi phút — dòng giá có `valid_from` / `valid_to` vừa qua
  từ lần chạy trước: dựng lại storefront, phát `catalog.price_effective`; `catalog.catch_up_embeddings`
  mỗi giờ; `catalog.cleanup_transfers` mỗi ngày (xoá file xuất cũ hơn 7 ngày).
- Task: `catalog.load_restrictions pack=vn` (nạp `modules/catalog/data/vn/restrictions.json` và bộ
  thuộc tính nhãn hàng hoá mẫu, ghi nguồn và ngày), `catalog.rebuild_storefront`.

### Cấu hình (`Setting`, tiền tố `AUTOXIDE_CATALOG_`)

`PRODUCT_PATH_PREFIX` (`/products`), `MAX_IMPORT_ROWS` (50000), `MAX_IMPORT_BYTES` (20 MiB),
`MAX_DIGITAL_FILE_BYTES` (500 MiB), `DOWNLOAD_LINK_SECONDS` (300), `DEFAULT_DOWNLOAD_LIMIT` (5),
`DEFAULT_GRANT_DAYS` (30). Ngôn ngữ và tiền tệ của cửa hàng lấy từ cài đặt kế thừa (bước 34), không
từ biến môi trường. Sinh lại `.env.example`.

### Tính năng và hạn mức (bước 35)

`Module::limits`: `catalog.products` (số sản phẩm chưa lưu trữ). `Module::features`:
`catalog.variants`, `catalog.price_lists`, `catalog.digital`, `catalog.bundles`, `catalog.ai`.

### Permission

`catalog.products.read`, `catalog.products.write`, `catalog.products.publish`, `catalog.prices.read`,
`catalog.prices.write`, `catalog.categories.manage`, `catalog.documents.manage`, `catalog.import`,
`catalog.export`, `catalog.downloads.manage`.

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `catalog.product_not_found` | 404 | Không tìm thấy sản phẩm. |
| `catalog.variant_not_found` | 404 | Không tìm thấy biến thể. |
| `catalog.category_not_found` | 404 | Không tìm thấy danh mục. |
| `catalog.price_list_not_found` | 404 | Không tìm thấy bảng giá. |
| `catalog.price_not_found` | 404 | Biến thể `{variant_id}` chưa có giá ở cửa hàng này. |
| `catalog.sku_taken` | 409 | Mã SKU `{sku}` đã dùng cho biến thể khác. |
| `catalog.barcode_taken` | 409 | Mã vạch `{barcode}` đã dùng cho biến thể khác. |
| `catalog.slug_taken` | 409 | Đường dẫn `{slug}` đã có sản phẩm khác dùng. |
| `catalog.option_combination_taken` | 409 | Sản phẩm đã có biến thể với tổ hợp tuỳ chọn này. |
| `catalog.invalid_status_transition` | 409 | Không chuyển được sản phẩm từ `{from}` sang `{to}`. |
| `catalog.required_attributes_missing` | 422 | Còn thiếu thuộc tính bắt buộc của danh mục: {attributes}. |
| `catalog.prohibited_goods` | 409 | Hàng thuộc danh mục cấm kinh doanh ({rule}), không bán được. |
| `catalog.documents_required` | 409 | Hàng kinh doanh có điều kiện cần giấy tờ còn hạn: {documents}. |
| `catalog.tax_category_unknown` | 422 | Nhóm thuế `{code}` không có trong engine thuế. |
| `catalog.consumer_price_rules` | 422 | Bảng giá bán cho người tiêu dùng ở Việt Nam phải bằng VND và đã gồm thuế. |
| `catalog.media_not_public` | 422 | Ảnh sản phẩm phải là media công khai. |
| `catalog.bundle_invalid` | 422 | Combo không chứa được combo khác, chính nó, hay dịch vụ. |
| `catalog.digital_asset_missing` | 409 | Hàng số phải có ít nhất một file trước khi mở bán. |
| `catalog.category_cycle` | 409 | Không dời được danh mục vào chính nhánh con của nó. |
| `catalog.category_not_empty` | 409 | Danh mục còn danh mục con hay sản phẩm. |
| `catalog.product_not_deletable` | 409 | Sản phẩm đã từng bán; hãy lưu trữ thay vì xoá. |
| `catalog.import_too_many_rows` | 422 | File có quá {max} dòng. |
| `catalog.download_not_found` | 404 | Link tải không còn hiệu lực. |
| `catalog.download_limit_reached` | 409 | Đã hết lượt tải cho link này. |

Lỗi theo dòng của việc nhập dùng chung khuôn `FieldError` (`field` = `row:<n>.<cột>`).

## Thứ tự làm

1. Chuẩn bị chung: `autoxide_kernel::csv`, `autoxide_kernel::slug`, `Subscription::optional` (+ test cả
   hai DB, ghi DECISIONS cho cả ba).
2. Khung crate từ `cargo xtask new-module catalog product`; đăng ký; hạn mức và tính năng.
3. Sản phẩm, bản dịch, tuỳ chọn, biến thể, SKU / mã vạch, ảnh, nhóm thuế, trạng thái → **mục 1**.
4. Danh mục cây, thuộc tính, thuộc tính bắt buộc theo danh mục, hàng cấm / có điều kiện + giấy tờ, task
   nạp gói "vn" → **mục 2**.
5. Bảng giá, dòng giá chỉ thêm, giải giá kế thừa, lịch sử, giá hẹn giờ, luật VND + gồm thuế → **mục 3**.
6. Storefront dẫn xuất và mọi endpoint công khai, ETag, sitemap, tìm → **mục 4**.
7. `api.rs` (`Catalog`), event, tool MCP → **mục 6**.
8. Nhập / xuất CSV bằng job; gợi ý AI thành bản nháp + `ai_assisted` → **mục 5**.
9. Hàng số (quyền tải, URL ký), dịch vụ, combo, sản phẩm tương tự → **mục 7**.
10. Tài liệu `docs/CATALOG.md` + `docs/CATALOG.vi.md`, sinh lại `docs/openapi.json`, `.env.example`,
    CHANGELOG, đánh dấu ROADMAP.

## Test bắt buộc

Mọi test qua `TestDatabase::run` / `TestApp::builder`, chạy SQLite và Postgres; `dyn Entitlements`,
`dyn TaxEngine`, `dyn OrgDirectory`, `dyn Media`, `dyn Storage`, `FakeTextModel`, `FakeEmbedder` cắm bằng
`.provide(…)` hay testkit.

- `sku_va_ma_vach_duy_nhat_trong_khach` (hai khách dùng cùng SKU được); `sku_dong_thoi_chi_mot_thang`
  (16 request cùng SKU → 1 thành công, 15 `catalog.sku_taken`).
- `chuyen_trang_thai_kiem_thuoc_tinh_bat_buoc`, `hang_cam_khong_mo_ban_duoc`,
  `hang_co_dieu_kien_can_giay_to_con_han`, `thuoc_tinh_bat_buoc_theo_ngay_hieu_luc`.
- `gia_cua_hang_ghi_de_gia_chuoi`, `gia_nhom_khach_thang_gia_chung`, `gia_hen_gio_va_het_han`,
  `gia_ngay_truoc_thoi_diem` (`reference_prices` lấy dòng < `at`, không lấy dòng = `at`),
  `dong_gia_khong_sua_duoc`, `bang_gia_nguoi_tieu_dung_vn_phai_vnd_gom_thue`,
  `them_gia_cung_reference_khong_tao_hai_lan` (16 request đồng thời).
- `doi_danh_muc_khong_tao_vong`, `doi_danh_muc_dong_thoi` (hai lần dời chéo nhau đồng thời: một bên
  `catalog.category_cycle` hay cả hai hợp lệ, cây không vòng).
- Công khai: `chi_tra_san_pham_dang_ban`, `loc_theo_thuoc_tinh_va_khoang_gia`, `tim_khong_dau`,
  `etag_doi_khi_anh_hay_gia_doi`, `sitemap_du_moi_duong_dan`, `cua_hang_an_san_pham_cua_chuoi`.
- Nhập CSV: `nhap_bao_loi_theo_dong` (dòng sai không chặn dòng đúng), `nhap_lai_cung_file_khong_nhan_doi`
  (khớp theo SKU), `xuat_chan_csv_injection`.
- Hàng số: `phat_quyen_tai_goi_lai_khong_phat_them`, `tai_dong_thoi_khong_vuot_luot` (20 request, hạn 5
  → đúng 5), `token_het_han_hay_thu_hoi`, `url_ky_cua_khach_a_bi_tu_choi_o_khach_b`.
- Combo: `combo_khong_long_combo`; AI: `goi_y_chi_la_ban_nhap`, `ap_goi_y_bat_ai_assisted`.
- Hạn mức: `tao_san_pham_vuot_han_muc` và `tao_loi_thi_tra_lai_han_muc`.
- **Rò rỉ**: `ro_ri_giua_khach` (bài chung của testkit cho mọi endpoint), `ro_ri_giua_hai_cua_hang_anh_em`
  (nhân viên cửa hàng A độ sâu `Unit` không thấy / sửa sản phẩm, giá, bảng giá của cửa hàng B).
- `api_catalog_qua_testapp` — gọi `app.api::<dyn Catalog>()` cho từng hàm của trait.
- Kernel: `csv_doc_ghi_khu_hoi`, `subscription_optional_khong_loi_khoi_dong`.

## Tiêu chí xong

- [ ] Mọi mục `- [ ]` của bước 41 trong ROADMAP được phủ theo bảng "Thứ tự làm".
- [ ] `cargo xtask check` xanh (hai mục công khai, tiền tố bảng, không handler chạm DB, mã lỗi có
      bản dịch); bảng cấp khách có `tenant_id` (testkit chặn).
- [ ] Mỗi endpoint có summary + `description` từng response; `docs/openapi.json`, `.env.example` sinh lại.
- [ ] `docs/CATALOG.md` + `docs/CATALOG.vi.md` cùng nội dung (cách khai giá kế thừa, thuộc tính bắt
      buộc, nhập CSV, hàng số); CHANGELOG mục **Added**.
- [ ] DECISIONS: ba API kernel mới; cách lưu giá chỉ thêm; storefront dẫn xuất; bộ đọc CSV; căn cứ pháp
      lý giá niêm yết, nhãn hàng hoá, hàng cấm (D-186).
- [ ] `./gate.sh` đầy đủ xanh trên SQLite và Postgres; đánh dấu ROADMAP; commit; phát hành theo
      `docs/RELEASING.md`.

## Chỗ dễ sai

- **Giải giá đọc nhiều dòng**: đừng gọi một câu cho mỗi biến thể; một câu lấy mọi dòng ứng viên của
  `variant_ids` (≤ 500) rồi chọn trong Rust. `OrgDirectory::ancestors` gọi một lần mỗi yêu cầu.
- **Thời điểm bằng nhau**: `reference_prices` dùng `valid_from < at`, `prices` dùng `<= at`. Postgres
  lưu micro giây, SQLite lưu chuỗi — cắt về micro giây trước khi lưu và so (như `tokens.rs`).
- **Unique nhiều NULL**: `barcode` NULL lặp được ở cả hai DB; chuỗi rỗng thì không — chuẩn hoá `""`
  thành NULL.
- **Slug**: unique theo (`tenant_id`, `locale`, `slug`) — không theo cửa hàng; hai cửa hàng của cùng
  khách không trùng slug được. Nếu cần khác thì ghi quyết định trước khi làm.
- **Hạn mức là giữ chỗ ngoài transaction**: `Entitlements::reserve` trước, transaction lỗi thì
  `release`; xoá sản phẩm nháp cũng `release`. Lưu trữ không trả hạn mức (sản phẩm vẫn còn).
- **Storefront dẫn xuất lệch**: mọi thay đổi ảnh hưởng (giá, trạng thái, ẩn hiện, dời nút tổ chức, giá
  hẹn giờ) phải xếp job trong cùng transaction; job dựng lại chạy lại được; có task dựng lại toàn bộ.
- **Lượt tải và `Range`**: trình tải chia nhiều request `Range` — chỉ trừ lượt ở `POST`, `GET …/file`
  không trừ. Token tải gửi cho người mua không lưu dạng đọc được (chỉ SHA-256).
- **Tìm theo ý nghĩa**: bộ nhớ đệm vector có khách trong khoá (bước 33, D-190); đừng chép nguyên
  `content::ai::semantic` — nếu thấy hai module cần cùng một chỉ mục trong bộ nhớ, đề xuất dời vào
  `autoxide_kernel::ai::vector`.
- **CSV nhập**: đọc theo luồng giới hạn cỡ / dòng; số tiền trong CSV là chuỗi số nguyên đơn vị nhỏ
  nhất (như JSON của `Money`), không nhận `1.000,50`.
- **Nhận event của `organizations`**: chịu được nhận lại (dựng lại là phép tính lại, không cộng dồn).

## Căn cứ pháp lý

Đối chiếu ngày 13/11/2026 qua nguồn tổng hợp — cần đối chiếu văn bản gốc / người có chuyên môn pháp
lý trước khi chạy thật (D-186):

- Niêm yết giá bằng Đồng Việt Nam, giá bán cho người tiêu dùng gồm thuế: Luật Giá 16/2023/QH15 (hiệu
  lực 01/7/2024), điều về niêm yết giá — cần đối chiếu điều khoản và trường hợp được niêm yết ngoại tệ.
- Nhãn hàng hoá (xuất xứ, tổ chức chịu trách nhiệm, hạn dùng, thành phần…): NĐ 43/2017/NĐ-CP, sửa bởi NĐ
  111/2021/NĐ-CP (hiệu lực 15/02/2022) — danh mục nội dung bắt buộc theo nhóm hàng là dữ liệu.
- Hàng cấm và ngành nghề kinh doanh có điều kiện: Luật Đầu tư 61/2020/QH14 (Điều 6, Phụ lục IV) và
  văn bản sửa đổi; hàng hoá không được kinh doanh trên môi trường điện tử theo Luật TMĐT 122/2025/QH15
  và nghị định hướng dẫn — cần đối chiếu danh mục hiện hành.
- Nội dung do AI gợi ý: Luật Trí tuệ nhân tạo 134/2025/QH15 (từ 01/3/2026) — dấu `ai_assisted` (bước 37).

## Agent làm bước này tự quyết

- Bộ đọc CSV tự viết (~80 dòng, như bộ ghi) hay crate `csv` — khuyến nghị crate `csv` nếu bộ tự viết
  quá 150 dòng; thêm thì ghi `STACK.md`.
- Định dạng `description`: chữ thuần có đoạn (an toàn, không HTML) hay dùng lại cấu trúc thân bài của
  `content` (phải dời định dạng đó vào kernel trước) — khuyến nghị chữ thuần + Markdown tối giản do giao
  diện hiển thị, không lưu HTML.
- Kiểm chữ số kiểm tra GTIN cho mã vạch: khuyến nghị chỉ kiểm khi mã toàn số dài 8 / 12 / 13 / 14.
- Chặn cứng hay chỉ cảnh báo luật "VND + gồm thuế" khi vùng thuế của khách không phải Việt Nam —
  khuyến nghị chỉ áp khi vùng thuế của nút là `VN`.
- Ghi mọi lựa chọn ở trên vào `docs/DECISIONS.md` bằng skill `record-decision`.
