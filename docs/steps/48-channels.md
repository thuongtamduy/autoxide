# Bước 48 — Bán đa kênh (`channels`)

> Lộ trình: `docs/ROADMAP.md` bước 48 · Thiết kế: `docs/research/business-foundation.md` mục 8 (bộ nối
> đối tác là module riêng), mục 9 · Quyết định: D-187, D-191, D-194 · Đặc tả liên quan:
> `docs/steps/45-orders.md` (`OrderIntake`), `docs/steps/42-…` (`inventory`)

## Mục tiêu

Người bán ở Việt Nam bán cùng lúc trên website của mình, Shopee, Lazada, TikTok Shop, Facebook / Zalo
mà vẫn **một kho, một danh sách sản phẩm, đơn từ mọi nơi về một chỗ**. Xong bước này: core có trait
`SalesChannel` để mỗi sàn / mạng xã hội là một module bộ nối riêng (làm khi có tài khoản thử, D-191);
liên kết biến thể với mã trên từng kênh và giá riêng theo kênh; đồng bộ tồn có phần để dành cho từng
kênh; đơn từ kênh vào `orders` với kênh nguồn, trừ kho chung, **không trùng khi kênh gửi lại**; báo cáo
theo kênh; đối soát tiền kênh trả với sổ cái. Bộ nối giả (`fake`) chạy trong test.

## Bắt đầu khi

- Bước 45 (`orders`) đã merge — dùng `OrderIntake::{import, external_transition}`, event `orders.*`.
- Sẵn từ trước: 33 (kho secret theo khách — token OAuth của từng tài khoản kênh), 34 (nút cửa hàng /
  kho), 35 (tính năng, hạn mức), 36 (`Ledger`), 41 (`catalog`), 42 (`inventory`), 44 (`shipping`).
- Song song được với 46, 47, 49.

## Phạm vi

Crate mới `modules/channels` (`autoxide-channels`), đăng ký sau `orders`. Bố cục: `accounts/`,
`listings/` (liên kết, giá theo kênh), `stock/` (phân bổ, đẩy tồn), `inbound/` (đơn vào, webhook,
kéo đơn), `fulfillment.rs` (đẩy trạng thái giao lên kênh), `settlements/`, `registry.rs` (sổ bộ nối),
cùng các file chuẩn (`lib.rs`, `api.rs`, `config.rs`, `permissions.rs`, `events.rs`, `entity/`,
`migration/`, `http/`, `tools.rs`, `tests/` — bộ nối giả ở `tests/fake_channel.rs`).

Sửa ngoài crate: `Cargo.toml`, `crates/app/Cargo.toml`, `crates/app/src/modules.rs`,
`docs/openapi.json`, `.env.example`, `CHANGELOG.md`, `docs/ROADMAP.md`. Nếu `inventory::api` (bước 42)
chưa có hàm phân bổ cứng (xem "Phân bổ tồn"), thêm vào `inventory::api` kèm test trên cả hai database
và ghi quyết định — sửa qua hợp đồng của module đó, không đọc bảng của nó.

NGOÀI phạm vi: bộ nối thật cho Shopee, Lazada, TikTok Shop, Facebook, Zalo (module riêng
`autoxide-channel-<tên>`, cần tài khoản đối tác — secret thật, dừng hỏi chủ dự án); chat với người mua
trên kênh; quảng cáo trên kênh.

## Thiết kế

### Trait và sổ bộ nối

`channels` công bố `dyn ChannelConnectors` trong `Module::start`; mỗi module bộ nối (đăng ký SAU
`channels`) gọi `context.require::<dyn ChannelConnectors>()?.register(Arc::new(ShopeeConnector…))`.
Một `kind` chỉ đăng ký một lần (trùng → lỗi khởi động). Tài khoản kênh (`channels_accounts`) chọn bộ nối
theo `kind`; `kind` chưa có bộ nối → tài khoản `unavailable`, không chặn phần còn lại.

```rust
// channels::api — phác thảo, tra API thật bằng skill `check-library-api`
#[async_trait]
pub trait SalesChannel: Send + Sync {
    fn kind(&self) -> &'static str;                       // "shopee", "lazada", "tiktok_shop", "facebook", "zalo", "fake"
    fn capabilities(&self) -> Capabilities;               // listings, price, stock, pull_orders, webhooks, fulfillment, cancel, settlements
    async fn verify(&self, account: &AccountContext) -> Result<AccountInfo, ChannelError>;
    async fn upsert_listing(&self, account: &AccountContext, listing: &ListingDraft) -> Result<ExternalListing, ChannelError>;
    async fn update_stock_and_price(&self, account: &AccountContext, updates: &[StockPriceUpdate]) -> Result<Vec<UpdateOutcome>, ChannelError>;
    async fn fetch_orders(&self, account: &AccountContext, cursor: Option<&str>) -> Result<OrderBatch, ChannelError>;
    /// Kiểm chữ ký và đọc thông báo đẩy của kênh; không xử lý nghiệp vụ.
    async fn parse_webhook(&self, account: &AccountContext, headers: &http::HeaderMap, body: &[u8]) -> Result<Vec<ChannelNotice>, ChannelError>;
    async fn update_fulfillment(&self, account: &AccountContext, external_order_id: &str, update: &FulfillmentUpdate) -> Result<(), ChannelError>;
    async fn cancel_order(&self, account: &AccountContext, external_order_id: &str, reason: CancelReason) -> Result<(), ChannelError>;
    async fn fetch_settlements(&self, account: &AccountContext, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<SettlementRecord>, ChannelError>;
}
pub trait ChannelConnectors: Send + Sync {
    fn register(&self, connector: Arc<dyn SalesChannel>) -> Result<(), Error>;
}
pub struct AccountContext { pub tenant_id: TenantId, pub account_id: Uuid, pub org_id: OrgId,
    pub credentials: SecretString, pub settings: serde_json::Value }      // #[non_exhaustive]
pub enum ChannelError { Unauthorized, RateLimited { retry_after: Duration }, Rejected { code: String, message: String },
    Unavailable, Unsupported }                                            // #[non_exhaustive]
pub struct ExternalOrder { pub external_id: String, pub external_status: String, pub status: ExternalStatus,
    pub placed_at: DateTime<Utc>, pub buyer: ExternalBuyer, pub ship_to: Option<ExternalAddress>,
    pub lines: Vec<ExternalLine /* external_item_id, external_sku, quantity, unit_price, discount */>,
    pub shipping_fee: Money, pub total: Money, pub payment: ExternalPayment /* Prepaid | CashOnDelivery */,
    pub fulfilled_by_channel: bool, pub raw: serde_json::Value }
pub enum ExternalStatus { Unpaid, ReadyToShip, Shipping, Delivered, Completed, Cancelled, Returned }
```

`ChannelError::RateLimited` → job đặt lại `run_at` theo `retry_after`; `Unauthorized` → tài khoản sang
`reauth_required`, báo quản trị (`Notifier`, tin giao dịch); bộ nối không bao giờ tự `panic`.
Credentials giải mã từ kho secret theo khách ngay trước khi gọi, không log, không nằm trong event.

### Liên kết sản phẩm và giá theo kênh

`channels_listings` ánh xạ (tài khoản, biến thể) ↔ (`external_item_id`, `external_sku`). Đăng sản phẩm
từ `catalog` (`Catalog::snapshot` + nội dung, ảnh) lên kênh bằng `upsert_listing` qua job. Giá theo
kênh: không đặt → giá bảng giá của cửa hàng gắn với tài khoản; đặt `price_rule` = giá cố định (`Money`,
có `effective_from`) hay tăng / giảm `markup_bp` trên giá catalog (làm tròn lên bậc `rounding_minor`, vd
1.000 đồng). Nghe `catalog.price_changed` / `catalog.product_status_changed` (giả định tên của bước
41) → enqueue đẩy giá / gỡ bán. Thuộc tính bắt buộc riêng của kênh (ngành hàng của kênh) lưu ở
`channel_attributes` (JSON, bộ nối kiểm).

### Phân bổ tồn

Tồn bán được của một tài khoản kênh = tồn khả dụng (đã trừ giữ chỗ) ở các kho gắn với tài khoản
(`warehouse_org_ids`), lấy qua `Inventory::available(tenant, variant, warehouses)` (giả định bước 42).
Mỗi (tài khoản, biến thể) — hay mặc định của tài khoản — có một quy tắc trong `channels_allocations`:

- `share` (mặc định): kênh được `floor((khả dụng − Σ phần cứng của mọi kênh) × share_bp / 10000) −
  buffer_qty`, không âm. Không giữ chỗ trong kho; `buffer_qty` bù độ trễ đồng bộ.
- `reserved`: kênh có phần cứng `reserved_qty` được **giữ thật trong kho** để website và kênh khác
  không bán vào — giữ chỗ dài hạn trong `inventory` với `reference = channels:alloc:<account>:<variant>`.
  Cần ở `inventory::api` (API mới nếu bước 42 chưa có):
  `set_allocation(tenant, reference, variant, warehouses, quantity) -> Result<u64 /* số giữ được */, Error>`
  và `consume_allocation(tenant, reference, order_reference, quantity) -> Result<u64, Error>` (chuyển
  phần giữ sang giữ chỗ của đơn trong một transaction của `inventory`). Đơn từ kênh đó tiêu phần cứng
  trước, phần thiếu lấy từ tồn chung.

**Không bán quá** trong Autoxide được bảo đảm bởi `inventory`: mọi đơn (web, POS, kênh) đều giữ chỗ
nguyên tử. Rủi ro còn lại là độ trễ của kênh — hai kênh cùng bán cái cuối trong khoảng chưa đồng bộ:
đơn nhập sau thành `stock_shortage` (bước 45), `channels` đánh dấu `oversold`, báo quản trị, và nếu
`auto_cancel_oversold` của tài khoản bật + bộ nối có `cancel` thì huỷ trên kênh với lý do hết hàng.
Tài liệu cho người dùng nói rõ điều này; `buffer_qty` và `reserved` là cách giảm.

Đẩy tồn: nghe `inventory.stock_changed` (giả định bước 42) → enqueue `channels.push_stock`
(`unique_key = push_stock:<account>:<variant>:<khe>`, `khe = floor(now / PUSH_DEBOUNCE_SECONDS)`,
`run_at` = cuối khe) — nhiều thay đổi trong cùng khe gộp thành một lần đẩy; job đọc tồn **lúc chạy**,
không lấy số trong payload; lô tối đa theo `capabilities.max_batch`. Job đối chiếu toàn bộ mỗi
`FULL_SYNC_HOURS` giờ. Đơn nhập từ kênh không cần đẩy lại tồn cho chính kênh đó (kênh đã tự trừ) nhưng
vẫn đẩy cho kênh khác.

### Đơn từ kênh

Hai đường vào, một đường xử lý: webhook (`POST /webhooks/{account_id}`, công khai, bộ nối kiểm chữ ký,
lưu nguyên văn vào `channels_webhook_receipts` rồi trả 200 ngay) và kéo định kỳ (`channels.pull_orders`
với con trỏ của tài khoản). Cả hai ghi `channels_inbound_orders` theo khoá unique (`tenant_id`,
`account_id`, `external_order_id`) — nhận lại chỉ cập nhật trạng thái kênh và `payload_hash`, không tạo
dòng mới. Job `channels.import_order`:

1. Ánh xạ từng dòng qua `channels_listings` (`external_item_id`, rồi `external_sku`); thiếu → trạng thái
   `needs_mapping`, event, chờ người ánh xạ (`POST /inbound-orders/{id}/mapping`) rồi chạy lại.
2. `OrderIntake::import(ImportedOrder { channel: "<kind>", channel_account_id, external_ref,
   org_id, lines (giá kênh, gồm thuế), shipping_fee, buyer contact, ship_to, payment, funds_flow:
   ChannelCollect, placed_at, fulfilled_by_channel })` — `orders` không trùng theo (`channel`,
   `external_ref`); kết quả `Replayed` là bình thường.
3. Lưu `order_id`, trạng thái `imported` (hay `oversold` nếu `orders.stock_shortage`).
4. Trạng thái kênh đổi sau đó (huỷ, đang giao, đã giao, hoàn tất, trả hàng) → `external_transition`
   theo bảng ánh xạ `ExternalStatus → OrderStatus`; chuyển không hợp lệ (vd kênh báo huỷ sau khi đã giao)
   → ghi `channels_inbound_orders.conflict`, event `channels.order_conflict`, người xử lý.

Đơn kênh mà người bán tự giao (`fulfilled_by_channel = false`): nghe `shipping.shipment_created` /
`orders.order_status_changed` của đơn có kênh nguồn → `update_fulfillment` (mã vận đơn, hãng) qua job.
Đơn kênh tự giao (`true`): `orders` không yêu cầu vận đơn; trạng thái giao đi từ kênh.

Dữ liệu người mua trong `raw`: giữ `RAW_RETENTION_DAYS` ngày rồi xoá trường `raw` (job), chỉ giữ
những gì đã chép vào đơn.

### Đối soát tiền kênh trả

`channels.fetch_settlements` (mỗi ngày) kéo bảng kê của kênh → `channels_settlements` (kỳ, mã chi trả,
tổng) + `channels_settlement_lines` (đơn, doanh thu, phí kênh từng loại, trợ giá, phí vận chuyển, thực
nhận). Khớp từng dòng với đơn (`external_order_id` → `order_id`) và số `orders` đã ghi; lệch → dòng
`mismatch` vào hàng chờ xử lý tay. Bút toán (`reference = channels:settlement:<line_id>`, qua job):

| Việc | Nợ | Có |
|---|---|---|
| Kênh trừ phí | `channel_fee_expense:<kind>` | `channel_receivable:<account>` |
| Kênh chi tiền về ngân hàng của người bán | `bank_clearing:<account>` | `channel_receivable:<account>` |
| Trợ giá của kênh | `channel_receivable:<account>` | `channel_subsidy_income` |

(Đơn nhập đã ghi Nợ `channel_receivable` / Có `buyer_prepayments` ở bước 45.) Số dư
`channel_receivable` của tài khoản sau đối soát = tiền kênh còn nợ — báo cáo
`GET /settlements/summary` so với sổ cái.

### Bảng dữ liệu

Quy ước như bước 45 (`tenant_id` đầu mọi khoá; UUIDv7; UTC; `Money` hai cột; JSON trong `text`;
migration có `down`; `outbox_migration`, `jobs_migration`).

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `channels_accounts` | `id`, `tenant_id`, `org_id` (cửa hàng sở hữu), `kind`, `name`, `external_shop_id` NULL, `credentials_ref` (kho secret), `warehouse_org_ids` text, `price_list_org_id` NULL, `settings` text (`auto_cancel_oversold`, `order_pull_minutes`…), `status` (`active/paused/reauth_required/unavailable/disconnected`), `order_cursor` NULL, `last_synced_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `kind`, `external_shop_id`); index (`tenant_id`, `org_id`) |
| `channels_listings` | `id`, `tenant_id`, `account_id`, `variant_id`, `external_item_id` NULL, `external_sku`, `channel_attributes` text, `price_rule` text NULL, `status` (`draft/pending/live/rejected/delisted/error`), `last_error` NULL, `last_pushed_price_*` NULL, `last_pushed_stock` NULL, `last_pushed_at` NULL, `updated_at` | unique (`tenant_id`, `account_id`, `variant_id`); unique (`tenant_id`, `account_id`, `external_sku`); index (`tenant_id`, `account_id`, `external_item_id`) |
| `channels_allocations` | `id`, `tenant_id`, `account_id`, `variant_id` NULL (NULL = mặc định), `mode` (`share/reserved`), `share_bp` INT, `reserved_qty` BIGINT, `buffer_qty` BIGINT, `updated_at` | unique (`tenant_id`, `account_id`, `variant_key`) |
| `channels_webhook_receipts` | `id`, `tenant_id`, `account_id`, `received_at`, `signature_ok` BOOL, `body` text, `notice_ids` text, `processed_at` NULL, `error` NULL | index (`tenant_id`, `account_id`, `received_at`) — xoá sau `RAW_RETENTION_DAYS` |
| `channels_inbound_orders` | `id`, `tenant_id`, `account_id`, `external_order_id`, `external_status`, `payload_hash`, `raw` text NULL, `status` (`received/needs_mapping/imported/oversold/conflict/failed`), `order_id` NULL, `conflict` text NULL, `attempts` INT, `received_at`, `updated_at` | unique (`tenant_id`, `account_id`, `external_order_id`); index (`tenant_id`, `status`, `updated_at`), (`tenant_id`, `order_id`) |
| `channels_settlements` | `id`, `tenant_id`, `account_id`, `external_payout_id`, `period_from`, `period_to`, `payout_*`, `status` (`imported/matched/mismatch/posted`), `created_at`, `updated_at` | unique (`tenant_id`, `account_id`, `external_payout_id`) |
| `channels_settlement_lines` | `id`, `tenant_id`, `settlement_id`, `external_order_id`, `order_id` NULL, `revenue_*`, `fees` text, `fees_total_*`, `subsidy_*`, `net_*`, `status` (`matched/mismatch/resolved`), `note` NULL | unique (`tenant_id`, `settlement_id`, `external_order_id`); index (`tenant_id`, `status`) |

### API REST (`/api/v1/channels`)

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /connectors` | `channels.read` | các `kind` đã đăng ký và `capabilities` |
| `GET /accounts` · `POST /accounts` · `GET/PUT /accounts/{id}` | `channels.read` / `channels.manage` | tạo tài khoản: credentials ghi vào kho secret, gọi `verify`; trả 201 |
| `POST /accounts/{id}/pause` · `/resume` · `/disconnect` | `channels.manage` | `disconnect` giữ lịch sử, xoá credentials |
| `GET /accounts/{id}/listings` · `PUT /accounts/{id}/listings/{variant_id}` · `POST /accounts/{id}/listings/{variant_id}/publish` | `channels.read` / `channels.manage` | liên kết mã, giá riêng, đăng / cập nhật |
| `POST /accounts/{id}/listings/import` | `channels.manage` | kéo danh sách sản phẩm có sẵn trên kênh, gợi ý khớp theo SKU / mã vạch (job) |
| `GET/PUT /accounts/{id}/allocations` · `PUT /accounts/{id}/allocations/{variant_id}` | `channels.manage` | quy tắc phân bổ |
| `POST /accounts/{id}/sync` | `channels.manage` | đẩy tồn / giá toàn bộ ngay (job) |
| `GET /accounts/{id}/stock-preview?variant_id=` | `channels.read` | số sẽ đẩy cho từng kênh, kèm cách tính |
| `POST /webhooks/{account_id}` | công khai (chữ ký của kênh) | luôn trả 200 sau khi lưu; chữ ký sai → 401 `channels.webhook_signature_invalid` |
| `GET /inbound-orders` · `GET /inbound-orders/{id}` | `channels.orders.manage` | lọc `needs_mapping`, `oversold`, `conflict`, `failed` |
| `POST /inbound-orders/{id}/mapping` · `POST /inbound-orders/{id}/retry` · `POST /inbound-orders/{id}/resolve` | `channels.orders.manage` | ánh xạ dòng, chạy lại, đóng xung đột |
| `GET /settlements` · `GET /settlements/{id}` · `POST /settlement-lines/{id}/resolve` · `GET /settlements/summary` | `channels.finance.read` / `channels.finance.manage` | đối soát |
| `GET /reports/channels?from&to` | `channels.read` + `orders.reports.read` | doanh thu theo kênh (gọi báo cáo của `orders` qua `api` hay dùng `group_by=channel`), phí, thực nhận |

### Tool MCP

`list_channel_accounts` (`read_only`), `channel_stock_preview` (`read_only`), `set_channel_allocation`,
`link_listing`, `list_inbound_orders` (`read_only`), `map_inbound_order`, `retry_inbound_order`,
`channel_settlement_summary` (`read_only`). Cùng service và permission như REST.

### Event

`channels.account_connected`, `channels.account_status_changed`, `channels.listing_published`,
`channels.listing_failed`, `channels.order_received`, `channels.order_imported`,
`channels.order_needs_mapping`, `channels.order_oversold`, `channels.order_conflict`,
`channels.settlement_imported`, `channels.settlement_mismatch`. Không có event cho mỗi lần đẩy tồn (dùng
số đo `channels_stock_pushes_total{kind, outcome}` — nhãn là tập nhỏ cố định).

Nghe: `inventory.stock_changed`, `catalog.price_changed`, `catalog.product_status_changed`,
`orders.order_status_changed`, `orders.stock_shortage`, `shipping.shipment_created`. Chịu nhận lại.

### Job và lịch

`channels.pull_orders` (mỗi phút: tài khoản tới hạn theo `order_pull_minutes`), `channels.process_webhook`
(enqueue), `channels.import_order` (enqueue, `unique_key = import:<account>:<external_id>:<payload_hash>` — lần gửi lại có nội dung mới vẫn được xử lý),
`channels.push_stock` / `channels.push_price` (enqueue, khoá theo khe thời gian), `channels.full_sync` (mỗi
`FULL_SYNC_HOURS`), `channels.publish_listing` (enqueue), `channels.push_fulfillment` (enqueue),
`channels.fetch_settlements` (mỗi ngày), `channels.renew_allocations` (mỗi giờ, chế độ `reserved`),
`channels.purge_raw` (mỗi ngày).

### Cấu hình

`AUTOXIDE_CHANNELS_PUSH_DEBOUNCE_SECONDS` (5), `AUTOXIDE_CHANNELS_FULL_SYNC_HOURS` (6),
`AUTOXIDE_CHANNELS_DEFAULT_PULL_MINUTES` (5), `AUTOXIDE_CHANNELS_RAW_RETENTION_DAYS` (30),
`AUTOXIDE_CHANNELS_IMPORT_MAX_ATTEMPTS` (10). Tính năng: `channels.multi_channel`,
`channels.reserved_allocation`. Hạn mức: `channels.accounts`.

### Permission

`channels.read`, `channels.manage`, `channels.orders.manage`, `channels.finance.read`,
`channels.finance.manage`.

### Mã lỗi

| Code | HTTP | Tiếng Việt |
|---|---|---|
| `channels.connector_unavailable` | 422 | Chưa cài bộ nối cho kênh {kind}. |
| `channels.account_not_found` | 404 | Không tìm thấy tài khoản kênh bán. |
| `channels.account_exists` | 409 | Cửa hàng {shop} trên {kind} đã được kết nối. |
| `channels.credentials_invalid` | 422 | Kênh {kind} từ chối thông tin kết nối: {detail}. |
| `channels.account_paused` | 409 | Tài khoản kênh đang tạm dừng. |
| `channels.capability_unsupported` | 422 | Kênh {kind} không hỗ trợ {capability}. |
| `channels.sku_taken` | 409 | Mã {sku} đã gắn với sản phẩm khác trên tài khoản này. |
| `channels.listing_rejected` | 422 | Kênh từ chối sản phẩm: {detail}. |
| `channels.allocation_invalid` | 422 | Phân bổ không hợp lệ: tổng tỉ lệ vượt 100% hoặc số âm. |
| `channels.allocation_unavailable` | 409 | Chỉ giữ được {held}/{requested} cho kênh do không đủ tồn. |
| `channels.webhook_signature_invalid` | 401 | Chữ ký thông báo của kênh không đúng. |
| `channels.inbound_not_found` | 404 | Không tìm thấy đơn nhận từ kênh. |
| `channels.mapping_incomplete` | 422 | Còn dòng chưa gắn sản phẩm: {lines}. |
| `channels.rate_limited` | 503 | Kênh {kind} đang giới hạn tần suất gọi; thử lại sau {seconds} giây. |
| `channels.settlement_line_not_found` | 404 | Không tìm thấy dòng đối soát. |

## Thứ tự làm

1. Khung crate, `api.rs` (`SalesChannel`, `ChannelConnectors`, kiểu dữ liệu), sổ bộ nối, bộ nối giả,
   tài khoản kênh + credentials trong kho secret. → **mục 1**
2. Liên kết sản phẩm / biến thể, giá theo kênh, đăng sản phẩm. → **mục 2**
3. Phân bổ tồn (`share`, `reserved` + API `inventory` nếu thiếu), đẩy tồn có debounce, đối chiếu toàn
   bộ. → **mục 2**
4. Đơn vào: webhook + kéo đơn, `channels_inbound_orders`, ánh xạ, `OrderIntake::import`,
   `external_transition`, quá bán, đẩy trạng thái giao. → **mục 3**
5. Đối soát tiền kênh trả + bút toán + báo cáo theo kênh. → **mục 3**
6. Tool MCP, `docs/openapi.json`, `.env.example`, hướng dẫn viết bộ nối kênh (EN + VI, vì người ngoài
   viết module bộ nối — thêm một mục vào `docs/MODULE_GUIDE.md` / `.vi.md`), CHANGELOG, ROADMAP.

## Test bắt buộc

Cả SQLite và Postgres; `TestApp` với `orders`, `inventory`, `catalog` thật và bộ nối `fake` (ghi lại
lệnh nhận được, tiêm lỗi `RateLimited` / `Unauthorized`, phát webhook có chữ ký).

- `connector_registers_once_and_unknown_kind_is_unavailable`.
- `credentials_never_in_events_logs_or_responses`.
- `channel_resends_same_order_creates_one_order` — cùng đơn qua webhook và kéo đơn, 10 lần song song:
  một dòng inbound, một đơn.
- `unmapped_line_waits_then_imports_after_mapping`.
- `stock_push_debounces_many_changes` — 50 thay đổi tồn trong 1 giây → 1 lần đẩy mỗi tài khoản.
- `share_allocation_math` (thuộc tính: không âm, Σ đẩy ≤ khả dụng khi không có `reserved`).
- `reserved_allocation_blocks_website_sale` — phần cứng của kênh không bán được trên website.
- `two_channels_sell_last_unit_one_oversold` — hai kênh nhập cùng lúc đơn cho cái cuối: một
  `imported`, một `oversold`, tồn không âm, có lệnh huỷ trên kênh khi bật.
- `channel_status_updates_order_through_lifecycle`; `invalid_channel_transition_becomes_conflict`.
- `settlement_matches_and_posts_ledger_once`; `settlement_mismatch_queued`.
- `rate_limited_connector_reschedules_job`; `unauthorized_marks_account_reauth_required`.
- `raw_payload_purged_after_retention`.
- Rò rỉ: `tenant_isolation`; `store_staff_sees_only_own_store_accounts`.

## Tiêu chí xong

- [ ] Ba mục của bước 48 có commit và test; gate đầy đủ xanh trên cả hai database.
- [ ] Bộ nối giả chạy mọi đường của trait; hướng dẫn viết bộ nối (EN + VI) đủ để người ngoài làm module
      bộ nối mà không đọc mã nguồn `channels`.
- [ ] Quyết định trong `docs/DECISIONS.md`: sổ bộ nối (đăng ký sau), hai chế độ phân bổ và giới hạn
      "không bán quá" khi kênh trễ, bút toán đối soát kênh; API `inventory` mới nếu có thêm.
- [ ] `docs/openapi.json`, `.env.example`, CHANGELOG, ROADMAP; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Không gọi kênh trong transaction**: mọi lệnh tới kênh đi qua job; transaction chỉ ghi trạng thái.
- **Đơn trùng**: khoá unique ở `channels_inbound_orders` và ở `orders` (`channel`, `external_ref`) —
  cả hai đều cần; `Replayed` không phải lỗi.
- **Thứ tự thông báo của kênh**: "đã giao" có thể tới trước "đang giao"; dựa vào bảng chuyển của
  `orders` (nhảy bước có ghi lịch sử) và `external_status` mới nhất theo thời điểm của kênh, không theo
  thời điểm nhận.
- **Debounce bằng `unique_key`**: khoá của kernel sống tới khi job đã xong bị dọn (`KEEP_COMPLETED` ở
  `crates/kernel/src/job/worker.rs`), nên khoá cố định `<account>:<variant>` sẽ chặn mọi lần đẩy sau —
  khoá phải mang khe thời gian như trên. Thay đổi tới khi job của khe đang chạy rơi vào khe sau, nên
  không mất cập nhật.
- **Giá kênh gồm thuế**: thuế tính bằng `TaxEngine` với `prices_include_tax = true` ở `orders`; không
  tin số thuế kênh trả về.
- **Múi giờ của kênh**: thời gian kênh trả có thể theo giờ địa phương không kèm múi — bộ nối phải đổi ra
  UTC; test với bộ nối giả trả giờ `+07:00`.
- **Dữ liệu người mua của kênh** bị che một phần (SĐT `*****123`): không dùng làm `buyer_key` chống bom
  hàng; đơn kênh không đưa vào thống kê COD của bước 45.

## Căn cứ pháp lý

- NĐ 117/2025/NĐ-CP: khi sàn ngoài (Shopee, Lazada, TikTok Shop) đã khấu trừ thuế thay người bán là hộ
  / cá nhân kinh doanh, bảng kê của kênh có phần thuế đã khấu trừ — lưu trong `fees` của dòng đối soát
  để người bán đối chiếu; cách kê khai phần doanh thu đã bị khấu trừ — **cần đối chiếu văn bản gốc**.
- Luật BVDLCN 91/2025 + NĐ 356/2025: dữ liệu người mua nhận từ kênh dùng để giao hàng và chăm sóc đơn;
  không dùng cho tiếp thị nếu không có đồng ý; thời hạn lưu dữ liệu thô là dữ liệu cấu hình.
- Luật Kế toán 88/2015: bảng kê đối soát và bút toán lưu ít nhất 10 năm.

## Agent làm bước này tự quyết

- Có hỗ trợ một sản phẩm kênh gộp nhiều biến thể / combo (một mã kênh = nhiều biến thể) không: khuyến
  nghị dùng combo của `catalog` (một biến thể combo) thay vì ánh xạ một-nhiều.
- Có tự huỷ đơn quá bán trên kênh mặc định không: khuyến nghị **tắt** (kênh phạt người bán huỷ đơn;
  người bán tự quyết nhập thêm hàng hay huỷ).
- Kéo đơn hay chỉ webhook khi kênh có cả hai: khuyến nghị cả hai (webhook nhanh, kéo đơn để không sót).
- Giá trị mặc định của debounce, chu kỳ đối chiếu — đo với bộ nối giả có độ trễ rồi chốt; ghi quyết định.

## Bổ sung theo D-204 (nguồn cấp sản phẩm cho Google, Meta)

Quảng cáo mua sắm của Google (Merchant Center) và danh mục sản phẩm của Meta (Facebook / Instagram) đọc
**nguồn cấp sản phẩm** (product feed) — một tệp do website công bố, họ tải định kỳ. Khác bộ nối sàn: không
nhận đơn, không cần tài khoản API, chỉ công bố dữ liệu công khai.

- **Bảng.** `channels_feeds`: `id`, `tenant_id`, `org_id` (cửa hàng), `format` (`google_merchant_xml` /
  `meta_csv`), `locale`, `currency`, `filter` JSON (danh mục, nhãn, loại trừ sản phẩm), `token_hash`,
  `media_id` NULL (tệp đã dựng, lưu qua `media` riêng tư hay `files`), `item_count`, `error_count`,
  `built_at` NULL, `created_at`, `updated_at`; index (`tenant_id`, `org_id`). `channels_feed_issues`:
  `tenant_id`, `feed_id`, `variant_id`, `code`, `field` (lỗi từng sản phẩm, vd thiếu GTIN, thiếu ảnh) — xoá
  và ghi lại mỗi lần dựng.
- **Dựng tệp bằng job** `channels.build_feed` (lịch theo cấu hình của nguồn, mặc định mỗi 6 giờ, và khi
  có event giá / tồn / trạng thái sản phẩm thì gom lại dựng một lần sau 15 phút): đọc qua `api` của
  `catalog` (tên, mô tả, ảnh, GTIN, thương hiệu, biến thể, đường dẫn — dữ liệu JSON-LD của D-201),
  `inventory` (mức công khai `in_stock` / `out` / `preorder` — không lộ số tồn), `promotions` (giá khuyến
  mại không cần mã và hạn của nó). Ghi theo lô, không giữ cả danh mục trong bộ nhớ. Chỉ sản phẩm
  `active`, công khai ở cửa hàng đó.
- **Phục vụ.** `GET /api/v1/channels/public/feeds/{id}.{xml|csv}?token=` (công khai, token dài, đổi được;
  `ETag`, `Last-Modified`, `Cache-Control`). Không có dữ liệu cá nhân hay giá vốn trong tệp.
- **Quản trị.** `GET/POST/PATCH/DELETE /feeds`, `POST /feeds/{id}/rotate-token`, `POST /feeds/{id}/build`,
  `GET /feeds/{id}/issues` (`channels.feeds.manage`). Tool MCP `list_feed_issues` (`read_only`),
  `build_feed`.
- **Đúng chuẩn.** Trường bắt buộc theo đặc tả của Google Merchant Center và Meta Commerce tại thời điểm làm
  (tra tài liệu chính thức, ghi phiên bản trong DECISIONS); giá kèm mã tiền tệ, tiền là số nguyên đơn vị
  nhỏ nhất đổi sang chuỗi đúng định dạng; giá gạch là giá ngay trước khuyến mại (Luật Giá, D-201 — cần
  đối chiếu).
- **Mã lỗi:** `channels.feed_not_found` (404 — cả khi token sai, không lộ có nguồn hay không),
  `channels.feed_format_unknown` (422), `channels.feed_building` (409 — đang dựng, thử lại sau).
- **Test** (cả hai database): `feed_chi_co_san_pham_cong_khai_cua_cua_hang`,
  `feed_khong_lo_so_ton_va_gia_von`, `feed_token_sai_tra_404`, `feed_bao_loi_tung_san_pham_thieu_gtin`,
  `feed_gia_khuyen_mai_kem_han`, `feed_dung_dinh_dang_xml_google` (so với tệp mẫu).

## Bổ sung theo D-222

- Giá theo kênh tính theo % trên giá cửa hàng = **bảng giá suy ra** của `catalog` có cột `channel` (đặc tả 41);
  `price_rule.markup_bp` và `rounding_minor` của bước này thay bằng tham chiếu `price_list_id` và
  `catalog_rounding_rules` — một cách làm. Giá cố định cho một tin đăng (`price_rule` = `Money`) giữ ở đây.

## Bổ sung theo đối chiếu thiết kế đợt 2 (D-223…D-227)

> Nguồn: `docs/prompts/ui-phase2-api.md`; màn tương ứng ghi trong `docs/research/ui-traceability.md` (đợt 2).

- **Trạng thái đơn kênh:** thêm `resolved` (người đóng xung đột hay đơn bán quá bằng `resolve`, kèm
  `resolution_code` — danh sách `channels.inbound_resolution`, D-223 — và ghi chú) và `cancelled_by_channel`
  (kênh huỷ trước khi nhập, hay sau khi nhập thì đơn của `orders` cũng huỷ). Đơn `oversold` có hàng lại và nhập được
  → `imported`.
- **Chế độ `reserved`:** số đẩy lên kênh = phần giữ riêng còn lại + phần chia theo tỉ lệ từ tồn chung (`share_bp`,
  mặc định 0 với `reserved`) − `buffer_qty`, không âm.
