# Bước 44 — Vận chuyển (`shipping`)

> Lộ trình: `docs/ROADMAP.md` bước 44 và phần "Giai đoạn module thương mại" · Thiết kế:
> `docs/research/business-foundation.md` mục 8 · Quyết định: D-180, D-184, D-186, D-187 (không giữ
> tiền), D-190 (secret theo khách), D-191, D-193 (`address`)

## Mục tiêu

Xong bước này, khách có sổ địa chỉ người mua theo đơn vị hành chính sau sáp nhập, tự đặt phí vận chuyển
(vùng theo tỉnh, bậc cân nặng / kích thước, miễn phí từ một giá trị đơn, nhận tại cửa hàng), cắm được
hãng vận chuyển qua trait `ShippingCarrier` (bộ nối thật là module riêng), quản lý vận đơn (nhiều kiện,
giao một phần, giao thất bại, hoàn hàng) và đối soát tiền thu hộ (COD) với bảng kê của hãng qua sổ cái.
`shipping` **không phụ thuộc `orders`**: nhận yêu cầu giao qua event, báo trạng thái qua event; `orders`
gọi `autoxide_shipping::api` để báo giá.

## Bắt đầu khi

- Bước 41 xong (`autoxide_kernel::csv`, `Subscription::optional`); số liệu cân nặng / kích thước do bên
  gọi đưa vào (lấy từ `VariantSnapshot` của `catalog`) — `shipping` không gọi `catalog`.
- Từ nền: `Address` và `dyn AdministrativeUnits` (`autoxide_address::api`, bước 34), `dyn OrgDirectory`,
  `TenantContext`, `Scope`, kho secret theo khách (bước 33), `dyn Ledger` (bước 36), người mua là tài khoản
  theo khách (bước 34).
- Song song được với 42, 43.

## Phạm vi

- Crate mới `modules/shipping` (`autoxide-shipping`), đăng ký sau `catalog` (cạnh `inventory`,
  `promotions`), trước `orders`.
- Ngoài phạm vi: bộ nối GHN, GHTK, Viettel Post, J&T… (module riêng `shipping-<hãng>` khi chủ dự án cấp
  tài khoản thử — dừng hỏi, D-191); vận chuyển quốc tế có thuế hải quan; tối ưu tuyến giao của đội xe
  riêng; trả hàng do người mua tự gửi (bước 45 tạo yêu cầu lấy hàng qua cùng cơ chế).

## Thiết kế

### Kiểu và trait

Phác thảo, tra API thật bằng skill `check-library-api`. `Address` là kiểu của bước 34.

```rust
// modules/shipping/src/api.rs

// ---- Báo giá, cho `orders` ----
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QuoteRequest {
    pub org_id: OrgId,                    // cửa hàng bán (phương thức kế thừa theo cây)
    pub origin_org_id: Option<OrgId>,     // kho xuất nếu đã biết (từ chỗ giữ của `inventory`)
    pub destination: Option<Address>,     // None: chỉ trả phương thức nhận tại cửa hàng
    pub items: Vec<QuoteItem>,
    pub order_value: Money,               // sau giảm giá, để xét miễn phí vận chuyển
    pub at: DateTime<Utc>,
    pub locale: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuoteItem {
    pub variant_id: Uuid, pub quantity: i64, pub requires_shipping: bool,
    pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>, pub width_mm: Option<i64>, pub height_mm: Option<i64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MethodKind { TableRate, Carrier, Pickup }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShippingOption {
    pub method_id: Uuid,
    pub method_code: String,
    pub name: String,
    pub kind: MethodKind,
    pub amount: Money,
    pub free_shipping_applied: bool,
    pub carrier_code: Option<String>,
    pub service_code: Option<String>,
    pub pickup_org_id: Option<OrgId>,
    pub chargeable_weight_grams: i64,
    pub estimated_days: Option<(u16, u16)>,
    pub option_key: String,               // dấu vân tay (phương thức, số tiền, đích) — `orders` gửi lại khi đặt
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Quote { pub options: Vec<ShippingOption>, pub unavailable: Vec<UnavailableMethod> }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnavailableMethod { pub method_code: String, pub reason_code: String }

#[async_trait::async_trait]
pub trait ShippingRates: Send + Sync {
    /// Các cách giao hợp lệ, rẻ trước. Hãng lỗi / quá giờ: phương thức đó vào `unavailable`,
    /// không làm hỏng cả báo giá.
    async fn quote(&self, tenant: &TenantContext, request: &QuoteRequest) -> Result<Quote, Error>;
}

// ---- Yêu cầu giao: `orders` nhúng kiểu này vào payload event (shipping không biết `orders`) ----
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ShipmentRequest {
    pub order_id: Uuid,
    pub order_number: String,
    pub request_key: String,              // chống trùng: một đơn nhiều lần giao (giao một phần)
    pub direction: Direction,             // Outbound | Return (lấy hàng trả từ người mua)
    pub org_id: OrgId,                    // cửa hàng bán
    pub origin_org_id: OrgId,             // kho xuất / kho nhận hàng trả
    pub method_id: Uuid,
    pub option_key: Option<String>,
    pub recipient_name: String,
    pub recipient_phone: String,
    pub address: Address,
    pub items: Vec<ShipmentItem>,
    pub declared_value: Money,
    pub cod_amount: Option<Money>,        // tiền hãng thu hộ khi giao
    pub note: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction { Outbound, Return }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShipmentItem {
    pub order_line_key: String, pub variant_id: Uuid, pub sku: String, pub name: String,
    pub quantity: i64, pub weight_grams: Option<i64>,
    pub length_mm: Option<i64>, pub width_mm: Option<i64>, pub height_mm: Option<i64>,
}

/// Trạng thái chung, ánh xạ từ trạng thái riêng của từng hãng.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShipmentStatus {
    Pending, Booking, ReadyToShip, PickedUp, InTransit, OutForDelivery, Delivered,
    DeliveryFailed, Returning, Returned, ReadyForPickup, PickedUpByBuyer, Cancelled, Lost,
}

// ---- Hãng vận chuyển: bộ nối là module riêng ----
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CarrierAccount {
    pub id: Uuid, pub carrier_code: String, pub sandbox: bool,
    pub settings: serde_json::Value,      // không bí mật
    pub credentials: SecretValue,         // giải mã từ kho secret theo khách (bước 33), không log
}
#[async_trait::async_trait]
pub trait ShippingCarrier: Send + Sync {
    fn code(&self) -> &'static str;       // `ghn`, `ghtk`, `fake`…
    fn capabilities(&self) -> CarrierCapabilities; // multi_parcel, cod, label, cancel, statement_api
    async fn quote(&self, account: &CarrierAccount, request: &CarrierQuoteRequest)
        -> Result<Vec<CarrierRate>, Error>;
    /// Tạo vận đơn. `request.client_reference` = ID vận đơn của Autoxide: hãng gọi lại cùng mã
    /// không tạo đơn thứ hai (hoặc bộ nối tự tra trước).
    async fn create_shipment(&self, account: &CarrierAccount, request: &CarrierShipmentRequest)
        -> Result<CarrierShipment, Error>;
    async fn find_by_reference(&self, account: &CarrierAccount, client_reference: &str)
        -> Result<Option<CarrierShipment>, Error>;
    async fn cancel(&self, account: &CarrierAccount, tracking_number: &str) -> Result<(), Error>;
    async fn label(&self, account: &CarrierAccount, tracking_number: &str) -> Result<Label, Error>;
    async fn track(&self, account: &CarrierAccount, tracking_number: &str)
        -> Result<Vec<CarrierUpdate>, Error>;
    /// Kiểm chữ ký webhook và đọc thân thành các cập nhật; chữ ký sai → lỗi, không đọc thân.
    fn parse_webhook(&self, account: &CarrierAccount, headers: &HeaderMap, body: &[u8])
        -> Result<Vec<CarrierUpdate>, Error>;
    /// Bảng kê COD theo kỳ, khi hãng có API (không có thì nhập CSV).
    async fn cod_statement(&self, account: &CarrierAccount, from: NaiveDate, to: NaiveDate)
        -> Result<Option<CodStatement>, Error> { Ok(None) }
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CarrierUpdate {
    pub event_id: String,                 // khoá chống trùng (hãng không có thì băm thân + thời gian)
    pub tracking_number: String,
    pub status: ShipmentStatus,
    pub carrier_status: String,           // nguyên văn của hãng
    pub occurred_at: DateTime<Utc>,
    pub cod_collected: Option<Money>,
    pub fee: Option<Money>,
    pub note: String,
}

/// Sổ hãng vận chuyển: bộ nối (module đăng ký SAU `shipping`) gọi `register` trong `Module::start`.
pub trait CarrierRegistry: Send + Sync {
    fn register(&self, carrier: Arc<dyn ShippingCarrier>) -> Result<(), Error>;
    fn get(&self, code: &str) -> Option<Arc<dyn ShippingCarrier>>;
    fn codes(&self) -> Vec<&'static str>;
}

/// Bộ giả cho test và để thử trên máy dev: trạng thái điều khiển bằng tay, ký webhook bằng HMAC.
pub mod fake { pub struct FakeCarrier { /* … */ } }
```

`shipping` công bố `dyn ShippingRates` và `dyn CarrierRegistry`. `fake::FakeCarrier` nằm trong `api`
(không tham chiếu `crate::…`) để testkit và bước 45 dùng; tự đăng ký khi
`AUTOXIDE_SHIPPING_FAKE_CARRIER=true` (mặc định tắt; `autoxide doctor` cảnh báo khi bật).

### Tính phí

- Phương thức kế thừa theo cây: phương thức của nút bán và tổ tiên (`OrgDirectory::ancestors`), nút gần
  hơn ghi đè cùng `code`.
- Hàng không cần giao (`requires_shipping = false`) không tính cân; giỏ toàn hàng không cần giao → không
  cần phương thức (báo giá rỗng, `orders` hiểu là không giao).
- Cân tính phí = max(tổng cân thật, tổng cân quy đổi) — cân quy đổi một món = D × R × C (cm³) /
  `volumetric_divisor` của phương thức (mặc định 6000), tính bằng số nguyên, làm tròn lên gram. Món
  thiếu cân nặng → phương thức `TableRate` dùng `default_item_weight_grams` của phương thức; không có thì
  vào `unavailable` (`shipping.weight_missing`).
- `TableRate`: vùng chứa tỉnh đích (vùng cụ thể theo tỉnh thắng vùng "cả nước"), bậc có
  `min_weight ≤ cân < max_weight`, cạnh dài nhất ≤ giới hạn bậc; phí bậc + `extra_per_step` cho phần vượt.
  `free_over` (Money) so với `order_value` → 0đ, `free_shipping_applied = true`.
- `Carrier`: gọi `ShippingCarrier::quote` (ngoài transaction, timeout); cộng `handling_fee`.
- `Pickup`: các điểm nhận (nút cửa hàng có cờ), phí 0 hay phí đặt.
- Địa chỉ đích phải hợp lệ theo `AdministrativeUnits` (xã thuộc tỉnh, còn hiệu lực); nước ngoài thì chỉ
  phương thức khai `countries` khác `VN`.

### Bảng dữ liệu

Mọi bảng có `tenant_id`; unique / index bắt đầu bằng nó; tiền `*_minor` + `*_currency`; cân / kích thước
`big_integer`; JSON trong `text`. Địa chỉ lưu bằng các cột của `Address` (quốc gia, mã tỉnh, mã xã, dòng
địa chỉ, tên tỉnh / xã lúc ghi; nước ngoài: `region`, `city`, `postal_code` — xem "Đề xuất" nếu `Address`
chưa có). Migration có `down`; thêm `outbox_migration`, `jobs_migration`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `shipping_addresses` | sổ địa chỉ người mua: `id`, `tenant_id`, `buyer_id`, `label`, `recipient_name`, `phone`, cột `Address`, `is_default`, `created_at`, `updated_at` | (`tenant_id`,`buyer_id`,`id`) |
| `shipping_zones` · `shipping_zone_regions` | `id`, `org_id`, `name` / `zone_id`, `country`, `province_code?` (NULL = cả nước) | (`tenant_id`,`org_id`); unique (`tenant_id`,`zone_id`,`country`,`province_code`) |
| `shipping_methods` | `id`, `tenant_id`, `org_id`, `code`, `kind`, `names` (JSON), `active`, `position`, `carrier_account_id?`, `service_code?`, `free_over_minor?`, `free_over_currency?`, `handling_fee_minor`, `currency`, `volumetric_divisor`, `default_item_weight_grams?`, `countries` (JSON), `estimated_min_days?`, `estimated_max_days?`, `updated_at` | unique (`tenant_id`,`org_id`,`code`) |
| `shipping_rates` | `method_id`, `zone_id`, `min_weight_grams`, `max_weight_grams?`, `max_side_mm?`, `amount_minor`, `currency`, `step_grams?`, `step_amount_minor?` | (`tenant_id`,`method_id`,`zone_id`,`min_weight_grams`) |
| `shipping_pickup_points` | `method_id`, `org_id` (cửa hàng), `instructions`, `prep_hours` | unique (`tenant_id`,`method_id`,`org_id`) |
| `shipping_carrier_accounts` | `id`, `tenant_id`, `org_id`, `carrier_code`, `name`, `secret_ref` (khoá trong kho secret theo khách), `webhook_token_hash`, `settings` (JSON), `sandbox`, `active` | (`tenant_id`,`carrier_code`) |
| `shipping_shipments` | `id`, `tenant_id`, `org_id`, `origin_org_id`, `order_id`, `order_number`, `request_key`, `request_fingerprint`, `direction`, `method_id`, `carrier_account_id?`, `carrier_code?`, `service_code?`, `tracking_number?`, `status`, `status_at`, `recipient_name`, `recipient_phone`, cột `Address`, `declared_value_minor`, `cod_minor?`, `currency`, `quoted_fee_minor?`, `actual_fee_minor?`, `cod_status` (`none`/`pending`/`collected`/`settled`/`mismatch`), `attempts`, `created_at`, `updated_at` | unique (`tenant_id`,`order_id`,`request_key`); unique (`tenant_id`,`carrier_code`,`tracking_number`); (`tenant_id`,`status`,`status_at`); (`tenant_id`,`org_id`,`created_at`,`id`) |
| `shipping_parcels` | `shipment_id`, `position`, `weight_grams`, `length_mm?`, `width_mm?`, `height_mm?`, `tracking_number?` | (`tenant_id`,`shipment_id`) |
| `shipping_shipment_items` | `shipment_id`, `parcel_id?`, `order_line_key`, `variant_id`, `sku`, `name`, `quantity` | (`tenant_id`,`shipment_id`) |
| `shipping_tracking_events` | **chỉ thêm**: `shipment_id`, `source` (`carrier`/`staff`/`system`), `event_key`, `status`, `carrier_status`, `occurred_at`, `note`, `created_at` | unique (`tenant_id`,`shipment_id`,`event_key`); (`tenant_id`,`shipment_id`,`occurred_at`) |
| `shipping_webhook_inbox` | nguyên văn webhook: `id`, `tenant_id`, `carrier_account_id`, `received_at`, `headers` (đã bỏ header bí mật), `body`, `status` (`received`/`processed`/`rejected`/`failed`), `error?` | (`tenant_id`,`status`,`received_at`) |
| `shipping_cod_statements` · `shipping_cod_statement_lines` | `id`, `carrier_account_id`, `statement_reference`, `period_from`, `period_to`, `total_cod_minor`, `total_fee_minor`, `net_paid_minor`, `currency`, `status` (`imported`/`reconciled`/`has_mismatch`), `imported_by`, `imported_at` / `tracking_number`, `cod_minor`, `fee_minor`, `shipment_id?`, `match` (`matched`/`amount_mismatch`/`unknown_tracking`/`not_delivered`) | unique (`tenant_id`,`carrier_account_id`,`statement_reference`) |
| `shipping_cod_discrepancies` | `statement_line_id?`, `shipment_id?`, `kind`, `expected_minor`, `reported_minor`, `status` (`open`/`resolved`), `resolution_note`, `resolved_by?` | (`tenant_id`,`status`) |

### API REST

Dưới `/api/v1/shipping`. Dữ liệu theo cửa hàng kiểm quyền theo độ sâu ở `org_id`.

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET/POST /me/addresses` · `PATCH/DELETE /me/addresses/{id}` · `POST /me/addresses/{id}/default` | `shipping.me.addresses` (người mua) | tối đa `MAX_ADDRESSES_PER_BUYER`; kiểm xã thuộc tỉnh |
| `GET /buyers/{buyer_id}/addresses` | `shipping.addresses.read` | cho nhân viên chăm sóc khách |
| `GET/POST /zones` · `PATCH/DELETE /zones/{id}` · `PUT /zones/{id}/regions` | `shipping.settings.manage` | |
| `GET/POST /methods` · `GET/PATCH/DELETE /methods/{id}` · `PUT /methods/{id}/rates` · `PUT /methods/{id}/pickup-points` | `shipping.settings.manage` | |
| `GET/POST /carrier-accounts` · `PATCH /carrier-accounts/{id}` · `POST /carrier-accounts/{id}/test` | `shipping.carriers.manage` | thông tin đăng nhập ghi vào kho secret, không bao giờ trả lại |
| `POST /quote` | `shipping.read` | thân `QuoteRequest`; cùng hàm với `api` (cho agent / nhân viên) |
| `GET /shipments?order_id&status&org_id&tracking` · `GET /shipments/{id}` | `shipping.shipments.read` | kèm kiện, món, lịch sử trạng thái |
| `POST /shipments` | `shipping.shipments.manage` | tạo tay (`ShipmentRequest`), dùng cho bán tại quầy giao sau |
| `POST /shipments/{id}/book` · `…/cancel` · `…/split` · `PUT /shipments/{id}/parcels` | `shipping.shipments.manage` | `book` gọi hãng ngoài transaction |
| `POST /shipments/{id}/status` | `shipping.shipments.manage` | cập nhật tay (giao bằng người của cửa hàng, nhận tại cửa hàng) |
| `GET /shipments/{id}/label` | `shipping.shipments.read` | file nhãn từ hãng, lưu đệm trong `Storage` (`shipping/labels/<id>`) |
| `POST /webhooks/{carrier_code}/{account_token}` | công khai | chỉ trả 2xx khi đã ghi nguyên văn; chữ ký sai → 401, không lưu thân |
| `POST /cod/statements` (CSV hay lấy qua API của hãng) · `GET /cod/statements[/{id}]` · `GET /cod/discrepancies` · `POST /cod/discrepancies/{id}/resolve` | `shipping.cod.manage` | `statement_reference` chống trùng |

Webhook: khách lấy từ `account_token` (token ngẫu nhiên, lưu băm) và cách phân giải khách giống webhook
cổng thanh toán của bước 36 — chốt theo đặc tả bước 33 / 36.

### Tool MCP

`shipping.quote` (`read_only`), `shipping.list_shipments`, `shipping.get_shipment`,
`shipping.book_shipment`, `shipping.cancel_shipment` (`destructive`), `shipping.update_status`,
`shipping.list_cod_discrepancies`.

### Event

Nhận bằng `Subscription::optional` (tên chốt với bước 45), payload `after.shipment` là `ShipmentRequest`:

| Event | Việc |
|---|---|
| `orders.fulfillment_requested` | tạo vận đơn `pending` (khoá `order_id` + `request_key`); phương thức có hãng và cửa hàng bật tự đặt → xếp job `shipping.book` |
| `orders.return_pickup_requested` | như trên với `direction = Return` |
| `orders.order_cancelled` | huỷ các vận đơn chưa lấy hàng của đơn (gọi hãng qua job) |

Phát (payload có `order_id`, `request_key`, `shipment_id`, `status`, `tracking_number`):
`shipping.shipment_created` · `shipping.shipment_booked` · `shipping.shipment_status_changed` ·
`shipping.shipment_delivered` · `shipping.shipment_delivery_failed` · `shipping.shipment_returned` ·
`shipping.shipment_cancelled` · `shipping.booking_failed` · `shipping.cod_collected` ·
`shipping.cod_settled` · `shipping.cod_mismatch` · `shipping.carrier_account_changed`.

### Job và lịch

- `shipping.book`: đổi `pending → booking` có điều kiện, gọi `create_shipment` (ngoài transaction,
  `client_reference` = ID vận đơn), ghi kết quả → `ready_to_ship`; lỗi tạm thì chạy lại (job retry); lỗi
  hẳn → `pending` + `shipping.booking_failed`.
- `shipping.process_webhook`: đọc `shipping_webhook_inbox`, áp từng `CarrierUpdate` (chèn
  `tracking_events` theo `event_key`, đổi trạng thái theo bảng chuyển hợp lệ).
- `shipping.post_cod`: khi giao xong có COD — ghi sổ cái Nợ "phải thu hãng `<account>`" / Có "phải thu
  người mua COD" (mã tham chiếu `shipping.cod:<shipment_id>`); khi đối soát khớp — Nợ "tiền đang chuyển
  về" (số thực trả) + Nợ "chi phí vận chuyển" (phí hãng trừ) / Có "phải thu hãng" (mã
  `shipping.cod_settle:<line_id>`). Tài khoản và chữ ký `Ledger` theo đặc tả bước 36 và 45.
- `shipping.import_cod_statement`: đọc CSV (`autoxide_kernel::csv`) / API của hãng, khớp theo
  `tracking_number`, sinh chênh lệch.
- Lịch: `shipping.reconcile_bookings` mỗi 5 phút (vận đơn kẹt ở `booking` quá 10 phút → hỏi
  `find_by_reference`); `shipping.poll_tracking` mỗi 30 phút cho hãng không có webhook; `shipping.cod_aging`
  mỗi ngày (giao xong quá `COD_SETTLEMENT_DAYS` mà chưa có trong bảng kê → chênh lệch `overdue`).

### Cấu hình (`AUTOXIDE_SHIPPING_`)

`FAKE_CARRIER` (false), `CARRIER_TIMEOUT_SECONDS` (10), `MAX_ADDRESSES_PER_BUYER` (20),
`WEBHOOK_MAX_BYTES` (256 KiB), `COD_SETTLEMENT_DAYS` (14). Cấu hình của hãng thuộc từng khách nằm
trong `shipping_carrier_accounts` + kho secret, không trong biến môi trường (D-190).

### Tính năng và hạn mức

`Module::features`: `shipping.carriers`, `shipping.cod`, `shipping.pickup`. `Module::limits`:
`shipping.carrier_accounts`.

### Permission

`shipping.read`, `shipping.me.addresses`, `shipping.addresses.read`, `shipping.settings.manage`,
`shipping.carriers.manage`, `shipping.shipments.read`, `shipping.shipments.manage`, `shipping.cod.manage`.

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `shipping.address_not_found` | 404 | Không tìm thấy địa chỉ. |
| `shipping.address_limit` | 409 | Mỗi người mua lưu tối đa {max} địa chỉ. |
| `shipping.invalid_address` | 422 | Địa chỉ không hợp lệ: {problem}. |
| `shipping.no_method` | 422 | Không có cách giao nào tới địa chỉ này cho giỏ hàng hiện tại. |
| `shipping.weight_missing` | 422 | Sản phẩm `{sku}` chưa có cân nặng để tính phí. |
| `shipping.method_not_found` / `zone_not_found` / `shipment_not_found` / `carrier_account_not_found` | 404 | Không tìm thấy phương thức giao / vùng / vận đơn / tài khoản hãng. |
| `shipping.method_code_taken` | 409 | Mã phương thức `{code}` đã có ở nút này. |
| `shipping.rate_overlap` | 422 | Các bậc cân nặng của vùng `{zone}` chồng nhau. |
| `shipping.carrier_unknown` | 422 | Chưa cài bộ nối cho hãng `{carrier}`. |
| `shipping.carrier_error` | 502 | Hãng vận chuyển báo lỗi: {message}. |
| `shipping.carrier_unavailable` | 503 | Hãng vận chuyển không trả lời; thử lại sau. |
| `shipping.invalid_transition` | 409 | Không chuyển được vận đơn từ `{from}` sang `{to}`. |
| `shipping.already_booked` | 409 | Vận đơn đã có mã vận đơn của hãng. |
| `shipping.cancel_too_late` | 409 | Hãng đã lấy hàng; không huỷ được vận đơn. |
| `shipping.webhook_signature` | 401 | Chữ ký webhook không hợp lệ. |
| `shipping.statement_duplicate` | 409 | Bảng kê `{reference}` đã nhập với nội dung khác. |
| `shipping.option_changed` | 409 | Phí vận chuyển đã thay đổi; hãy chọn lại cách giao. |

## Thứ tự làm

1. Khung crate, permission, tính năng / hạn mức, `CarrierRegistry`, `fake::FakeCarrier`.
2. Sổ địa chỉ người mua (`Address`, kiểm bằng `AdministrativeUnits`, địa chỉ nước ngoài dạng chung,
   `PersonalData` + xoá khi người mua bị xoá) → **mục 1**.
3. Vùng, phương thức, bậc phí, miễn phí, nhận tại cửa hàng, `ShippingRates::quote`, `POST /quote` →
   **mục 2**.
4. Tài khoản hãng (secret theo khách), `ShippingCarrier`, đặt / huỷ / nhãn / theo dõi, webhook (ghi
   nguyên văn → job, chống trùng, ánh xạ trạng thái) → **mục 3**.
5. Vận đơn: nhận event của `orders` (test bằng module giả), nhiều kiện, tách, giao thất bại, hoàn hàng,
   event phát ra → **mục 4**.
6. COD: ghi sổ cái, nhập bảng kê, khớp, chênh lệch, quá hạn → **mục 5**.
7. `docs/SHIPPING.md` + `.vi.md` (cách viết bộ nối hãng có ví dụ từ `FakeCarrier`), OpenAPI,
   `.env.example`, CHANGELOG, ROADMAP.

## Test bắt buộc

Cả SQLite và Postgres; `dyn AdministrativeUnits`, `dyn OrgDirectory`, `dyn Ledger`, kho secret bằng bộ giả;
hãng bằng `FakeCarrier`.

- Địa chỉ: `xa_phai_thuoc_tinh`, `ma_xa_het_hieu_luc_bi_tu_choi`, `dia_chi_nuoc_ngoai`,
  `mot_dia_chi_mac_dinh` (đặt mặc định đồng thời hai địa chỉ → còn đúng một), `xuat_va_xoa_du_lieu_ca_nhan`.
- Phí: `vung_theo_tinh_thang_ca_nuoc`, `bac_can_nang_va_quy_doi`, `mien_phi_tu_gia_tri_don`,
  `phuong_thuc_cua_hang_ghi_de_chuoi`, `hang_khong_can_giao`, `hang_loi_khong_lam_hong_bao_gia`
  (FakeCarrier quá giờ → `unavailable`), `bac_chong_nhau_bi_tu_choi`.
- Vận đơn: `nhan_lai_event_khong_tao_hai_van_don` (giao event hai lần + 16 lần đồng thời),
  `dat_hang_mat_ket_noi_giua_chung` (FakeCarrier tạo xong nhưng trả lỗi → `reconcile_bookings` tìm
  lại theo `client_reference`, không tạo đơn thứ hai), `giao_mot_phan_hai_van_don`,
  `nhieu_kien_mot_van_don`, `giao_that_bai_roi_hoan_hang`, `huy_sau_khi_lay_hang_bi_tu_choi`.
- Webhook: `chu_ky_sai_401_khong_luu`, `webhook_trung_chi_ap_mot_lan`, `webhook_den_sai_thu_tu`
  (`delivered` tới trước `in_transit` → trạng thái cuối vẫn `delivered`), `token_cua_khach_a_o_khach_b`.
- COD: `giao_xong_ghi_phai_thu_hang_mot_lan` (job chạy hai lần), `bang_ke_khop_ghi_so_cai`,
  `bang_ke_lech_tien_sinh_chenh_lech`, `ma_van_don_la_trong_bang_ke`, `nhap_lai_bang_ke_khong_nhan_doi`,
  `cod_qua_han_chua_doi_soat`.
- **Rò rỉ**: bài chung giữa khách (vận đơn, địa chỉ, tài khoản hãng); `ro_ri_giua_hai_cua_hang_anh_em`
  (nhân viên cửa hàng A không thấy vận đơn / phương thức riêng của B); người mua X không đọc được địa
  chỉ của người mua Y.
- `api_shipping_qua_testapp` (`ShippingRates`, `CarrierRegistry`), `fake_carrier_bi_canh_bao_trong_doctor`.

## Tiêu chí xong

- [ ] Mọi mục của bước 44 trong ROADMAP được phủ.
- [ ] `shipping` không có phụ thuộc Cargo vào `autoxide-orders` hay `autoxide-catalog`; chỉ biết tên
      event dạng chuỗi.
- [ ] Không gọi mạng trong transaction ghi; không lưu / log thông tin đăng nhập hãng.
- [ ] `docs/SHIPPING.md` + `docs/SHIPPING.vi.md`; OpenAPI, `.env.example`; CHANGELOG.
- [ ] DECISIONS: `CarrierRegistry` cho nhiều hãng; bảng trạng thái chung và chuyển hợp lệ; cân quy
      đổi; COD ghi sổ cái (core không giữ tiền, căn cứ); webhook ghi nguyên văn rồi xử lý bằng job.
- [ ] `./gate.sh` đầy đủ xanh; ROADMAP; commit; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Gọi hãng giữa hai transaction**: đặt `booking` (commit) → gọi hãng → ghi kết quả (commit). Chết
  giữa chừng thì vận đơn kẹt `booking`; `reconcile_bookings` hỏi lại bằng `client_reference` — không bao
  giờ gọi `create_shipment` lần hai mà chưa hỏi.
- **Webhook**: trả 2xx chỉ sau khi đã ghi nguyên văn; xử lý trong job; chống trùng bằng `event_key`;
  trạng thái chỉ đi tới theo bảng chuyển (so `occurred_at`, bỏ cập nhật cũ hơn trạng thái hiện tại trừ các
  chuyển cho phép như `delivery_failed → out_for_delivery`). Thân lớn quá `WEBHOOK_MAX_BYTES` → 413.
- **Thời gian của hãng** thường là giờ Việt Nam không kèm múi giờ — bộ nối đổi sang UTC.
- **Nhận event của `orders` hai lần, hoặc trước khi bước 45 tồn tại**: `Subscription::optional`;
  unique (`order_id`, `request_key`) + `request_fingerprint` (khác nội dung → ghi lỗi, không ghi đè).
- **COD không phải tiền của core**: chỉ ghi nghĩa vụ; số tiền COD lấy từ yêu cầu giao, không từ webhook
  (webhook có thể sai) — lệch thì vào chênh lệch.
- **Sổ cái là module khác**: ghi bằng job sau commit với mã tham chiếu; không gọi `Ledger` trong
  transaction của `shipping`.
- **Địa chỉ chụp lại**: vận đơn lưu tên tỉnh / xã lúc tạo; sửa sổ địa chỉ không đổi vận đơn cũ.
- **Hai database**: bậc cân `max_weight_grams` NULL = vô hạn — viết điều kiện `IS NULL OR >`, không
  dùng `COALESCE` với số lớn; tìm vùng theo `province_code` NULL dùng `IS NULL`, không `= NULL`.
- **Số điện thoại** người nhận là dữ liệu cá nhân: không đưa vào log, event chỉ mang ID.

## Căn cứ pháp lý

Đối chiếu ngày 13/11/2026 — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý (D-186):

- Địa chỉ: danh mục đơn vị hành chính hai cấp theo QĐ 19/2025/QĐ-TTg, hiệu lực 01/7/2025 (module
  `address`, D-193).
- Thu hộ (COD): hãng vận chuyển thu tiền và chuyển trả người bán theo hợp đồng giữa hai bên; core chỉ ghi
  nghĩa vụ và đối soát, không nhận hay giữ tiền (D-187; NĐ 52/2024/NĐ-CP về thanh toán không dùng tiền
  mặt — cần đối chiếu phạm vi dịch vụ thu hộ của doanh nghiệp bưu chính). Dịch vụ bưu chính: Luật Bưu
  chính 49/2010/QH12.
- Sổ địa chỉ là dữ liệu cá nhân: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và NĐ 356/2025 (từ 01/01/2026)
  — mục đích giao hàng, xuất và xoá theo yêu cầu, không dùng cho quảng cáo.
- Chứng từ đối soát COD là chứng từ kế toán: lưu theo Luật Kế toán 88/2015 (bước 36).

## Agent làm bước này tự quyết

- Vận đơn cho phương thức nhận tại cửa hàng: tạo vận đơn `ready_for_pickup` (khuyến nghị — một chỗ theo
  dõi mọi cách giao) hay để `orders` tự lo.
- Tự đặt vận đơn với hãng khi nhận yêu cầu: cờ theo phương thức, mặc định tắt (nhân viên kiểm hàng trước).
- Đọc bảng kê CSV theo một định dạng chung (cột `tracking_number`, `cod_amount`, `fee`) và để bộ nối
  từng hãng đổi định dạng riêng — khuyến nghị vậy.
- Ghi vào `docs/DECISIONS.md` (skill `record-decision`).

## Bổ sung theo D-200 (thiết kế giao diện 2)

**Ước tính phí giao công khai.** `GET /public/shipping/estimate?product_id&variant_id&quantity&province_code`
(công khai) chạy cùng `quote` với một giỏ ảo một dòng, giao tới mã tỉnh (chưa cần xã), trả các phương thức,
phí, thời gian dự kiến, ngưỡng miễn phí. Response ghi rõ `estimate: true`; phí thật luôn tính lại khi đặt
hàng. Cache ngắn theo (sản phẩm, tỉnh). Test: kết quả bằng `quote` của giỏ một dòng cùng tỉnh.

## Bổ sung theo D-201

**Ngày giao dự kiến.** `Quote` và ước tính công khai trả `eta: {earliest_date, latest_date, order_by}`
(ngày tuyệt đối theo múi giờ của khách, `order_by` = giờ chốt hôm nay nếu còn). Ngày làm việc, giờ chốt
theo phương thức, ngày lễ là dữ liệu có ngày hiệu lực (`shipping_calendars`). Nhận tại cửa hàng trả
`ready_at` = giờ đặt + `prep_hours` lùi tới giờ mở cửa kế tiếp (hồ sơ công khai của nút, bước 34) và
`hold_days`. Khoảng dài hơn 7 ngày thì ghi rõ lý do (hàng đặt trước, vùng xa).

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **GTGT trên phí giao (B-b5, B-c9).** `shipping_methods.tax_category`; báo giá trả dòng thuế cho phí giao,
  `orders` ghi dòng thuế đó lên hoá đơn. Căn cứ Luật Thuế GTGT 48/2024/QH15 — **cần đối chiếu văn bản gốc**.
- **COD (D-216).** Event `shipping.cod_settled` mang `seller_org_id` và tài khoản nhận (đối tác thu hộ hay
  người bán), để `ledger` ghi đúng chỗ.
