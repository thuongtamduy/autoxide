# Bước 42 — Kho (`inventory`)

> Lộ trình: `docs/ROADMAP.md` bước 42 và phần "Giai đoạn module thương mại" · Thiết kế:
> `docs/research/business-foundation.md` mục 8, 9 · Quyết định: D-180, D-181 (`rows`), D-182
> (`write_with_retry`), D-184 (`reference`), D-185, D-191, D-194

## Mục tiêu

Xong bước này, mỗi khách quản lý tồn theo biến thể × kho (kho là nút tổ chức loại `warehouse` hay
`store`), mọi biến động nằm trong một sổ chỉ thêm, và **không bao giờ bán quá tồn** dù nhiều request
cùng lúc trên SQLite hay Postgres. `orders` (45) giữ chỗ → xác nhận → xuất / trả lại theo `order_id`
qua `autoxide_inventory::api`; chuỗi bán lẻ chuyển kho, kiểm kê, nhập hàng từ nhà cung cấp, theo dõi lô
/ hạn dùng / serial và giá vốn bình quân cho báo cáo lãi gộp.

## Bắt đầu khi

- Bước 41 xong: `dyn Catalog` (biến thể, cờ `track_inventory` / `allow_backorder` / `lot_tracked` /
  `serial_tracked`, `bundle_components`), `autoxide_kernel::csv`, `Subscription::optional`.
- Từ nền: `TenantContext`, `OrgId`, `Scope`, `dyn OrgDirectory` (loại nút, trạng thái, `Address` của
  nút), `dyn Ledger` (bước 36), đánh số chứng từ `numbering` (bước 36), `dyn Notifier` (bước 37) không
  bắt buộc — cảnh báo tồn thấp đi qua event.
- Làm song song được với 43, 44.

## Phạm vi

- Crate mới `modules/inventory` (`autoxide-inventory`), đăng ký sau `catalog`.
- Ngoài phạm vi: tồn của người bán trong kho của sàn (46 — dùng lại bảng này với `owner_org_id`, chưa
  làm ở đây), phần dành cho từng kênh (48), bút toán giá vốn hàng bán vào sổ cái (chỉ ghi công nợ nhà
  cung cấp và giá trị nhập theo ROADMAP; ghi giá vốn khi bán là quyết định của 45 / về sau), hoá đơn đầu
  vào của nhà cung cấp.

## Thiết kế

### Mô hình tồn

Mỗi dòng `inventory_levels` (biến thể × kho) giữ hai số, đều đổi bằng `rows::adjust`:

- `on_hand` — hàng đang nằm trong kho = **tổng `quantity` của sổ biến động** của dòng đó.
- `available` — bán được = `on_hand − tổng chỗ đang giữ`. `reserved` không lưu, suy ra
  (`on_hand − available`) và trả trong API.

| Việc | `on_hand` | `available` | Sổ biến động |
|---|---|---|---|
| Giữ chỗ | — | −q (`Bounds::NON_NEGATIVE`; `Bounds::ANY` khi cho bán khi hết) | không |
| Trả chỗ (huỷ, hết hạn) | — | +q | không |
| Xuất (giao, bán tại quầy) | −q | — (đã trừ lúc giữ) | `sale` |
| Nhập / trả về / chuyển đến | +q | +q | `receipt` / `return` / `transfer_in` |
| Chuyển đi | −q | −q | `transfer_out` |
| Điều chỉnh, kiểm kê, hết hạn lô | ±q | ±q | `adjustment` / `count` / `expiry` |

Giữ chỗ là MỘT câu `UPDATE … SET available = available − q WHERE id = ? AND available >= q` — không
đọc trước. Hai cột cùng đổi trong một transaction (hai câu `adjust`); dòng đã bị câu đầu khoá trên
Postgres, SQLite chỉ một bên ghi.

### Kiểu và trait

Phác thảo, tra API thật bằng skill `check-library-api`.

```rust
// modules/inventory/src/api.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReserveRequest {
    pub order_id: Uuid,                    // mã tham chiếu: một đơn một lần giữ
    pub selling_org_id: OrgId,             // cửa hàng / kênh bán
    pub fixed_org_id: Option<OrgId>,       // nhận tại cửa hàng, bán tại quầy: chỉ lấy từ kho này
    pub ship_to_province: Option<String>,  // mã tỉnh (module `address`) để ưu tiên kho gần
    pub expires_at: DateTime<Utc>,         // hạn thanh toán do `orders` đặt (≤ MAX_HOLD_HOURS)
    pub lines: Vec<ReserveLine>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveLine { pub line_key: String, pub variant_id: Uuid, pub quantity: i64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReservationStatus { Held, Confirmed, Fulfilled, Released, Expired }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Reservation {
    pub id: Uuid, pub order_id: Uuid, pub status: ReservationStatus,
    pub expires_at: Option<DateTime<Utc>>,  // None khi đã xác nhận
    pub lines: Vec<ReservedLine>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservedLine {
    pub line_key: String,
    pub variant_id: Uuid,                  // biến thể thật bị trừ (thành phần, khi dòng là combo)
    pub bundle_variant_id: Option<Uuid>,
    pub warehouse_org_id: OrgId,
    pub quantity: i64,
    pub fulfilled_quantity: i64,
    pub backordered: bool,                 // giữ khi `available` âm (sản phẩm cho bán khi hết)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FulfilRequest {
    pub order_id: Uuid,
    pub fulfilment_key: String,            // vd mã vận đơn / lần xuất; chống trùng
    pub lines: Vec<FulfilLine>,            // trống = xuất hết phần còn lại
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulfilLine {
    pub line_key: String, pub quantity: i64,
    pub lot_code: Option<String>,          // trống + `lot_tracked` = tự chọn lô hết hạn trước
    pub serials: Vec<String>,              // bắt buộc khi `serial_tracked`
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RestockRequest {
    pub reference: String,                 // vd `return:<return_id>`; chống trùng vĩnh viễn
    pub order_id: Option<Uuid>,
    pub warehouse_org_id: OrgId,
    pub lines: Vec<RestockLine>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestockLine {
    pub variant_id: Uuid, pub quantity: i64,
    pub sellable: bool,                    // false: ghi nhận, không cộng vào tồn bán được
    pub lot_code: Option<String>, pub serials: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Availability {
    pub variant_id: Uuid, pub warehouse_org_id: OrgId,
    pub on_hand: i64, pub available: i64, pub incoming: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseReason { Cancelled, PaymentFailed, Expired, CheckoutAborted }

#[async_trait::async_trait]
pub trait Inventory: Send + Sync {
    /// Tồn bán được của các biến thể ở các kho phục vụ `selling_org_id` (cộng dồn và theo kho).
    async fn availability(&self, tenant: &TenantContext, selling_org_id: OrgId, variant_ids: &[Uuid])
        -> Result<Vec<Availability>, Error>;
    /// Giữ chỗ cho cả đơn: tất cả hoặc không gì. Gọi lại cùng `order_id` + cùng nội dung →
    /// `Submitted::Replayed`; khác nội dung → `reference_reused`. Combo được tách thành thành phần.
    async fn reserve(&self, tenant: &TenantContext, request: &ReserveRequest)
        -> Result<Submitted<Reservation>, Error>;
    /// Giữ → xác nhận (bỏ hạn). Đã xác nhận: trả như cũ. Đã hết hạn / trả: `inventory.reservation_expired`.
    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Reservation, Error>;
    /// Trả phần chưa xuất. Không có chỗ giữ, hay đã trả: `Ok(None)` / trả như cũ.
    async fn release(&self, tenant: &TenantContext, order_id: Uuid, reason: ReleaseReason)
        -> Result<Option<Reservation>, Error>;
    /// Xuất kho theo chỗ đã xác nhận (một phần được). Gọi lại cùng `fulfilment_key`: trả như cũ.
    async fn fulfil(&self, tenant: &TenantContext, request: &FulfilRequest) -> Result<Reservation, Error>;
    /// Nhập lại hàng trả về. Gọi lại cùng `reference` + cùng nội dung: không cộng lần hai.
    async fn restock(&self, tenant: &TenantContext, request: &RestockRequest) -> Result<(), Error>;
}
```

### Chọn kho

Ứng viên = `inventory_sources` của `selling_org_id` (bật, theo `priority`); không có dòng nào thì chính
`selling_org_id` nếu nó là `warehouse` / `store`; không có gì → `inventory.no_source`. Có
`fixed_org_id` thì chỉ kho đó. Sắp: kho cùng tỉnh với `ship_to_province` (tỉnh trong `Address` của
nút, hỏi `OrgDirectory`) trước khi `prefer_nearby` bật → `priority` → `id`. Thử từng kho đủ **mọi
dòng**; không kho nào đủ thì, khi `allow_split` bật, chia tham lam theo thứ tự; không thì
`inventory.insufficient_stock` (nêu từng `line_key`, số còn). `adjust` trả 0 ở kho đang thử thì
huỷ các phần đã trừ trong transaction đó (rollback toàn bộ rồi thử kho kế bằng transaction mới, hoặc
dùng savepoint — xem "Chỗ dễ sai"). Hai cờ `prefer_nearby`, `allow_split` lưu theo nút bán trong
`inventory_settings`.

### Bảng dữ liệu

Mọi bảng có `tenant_id`; unique / index bắt đầu bằng `tenant_id`; số lượng `big_integer`; tiền
`*_minor` + `*_currency`; thời gian UTC; JSON trong `text`. Migration có `down`; thêm
`outbox_migration`, `jobs_migration`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `inventory_levels` | `id`, `tenant_id`, `org_id` (kho), `variant_id`, `on_hand`, `available`, `incoming`, `reorder_point?`, `allow_negative` (chép từ `allow_backorder`), `average_cost_minor`, `average_cost_currency`, `updated_at` | unique (`tenant_id`,`org_id`,`variant_id`); (`tenant_id`,`variant_id`) |
| `inventory_movements` | **chỉ thêm**: `id`, `tenant_id`, `org_id`, `variant_id`, `level_id`, `kind` (`receipt`/`sale`/`return`/`adjustment`/`count`/`transfer_out`/`transfer_in`/`expiry`/`supplier_return`), `quantity` (có dấu), `unit_cost_minor`, `unit_cost_currency`, `lot_id?`, `reason_code?`, `note`, `source_type`, `source_id`, `actor_id?`, `created_at` | (`tenant_id`,`level_id`,`id`); (`tenant_id`,`source_type`,`source_id`); (`tenant_id`,`created_at`,`id`) |
| `inventory_reservations` | `id`, `tenant_id`, `order_id`, `selling_org_id`, `status`, `expires_at?`, `fingerprint`, `created_at`, `updated_at` | unique (`tenant_id`,`order_id`); (`tenant_id`,`status`,`expires_at`) |
| `inventory_reservation_lines` | `id`, `tenant_id`, `reservation_id`, `line_key`, `variant_id`, `bundle_variant_id?`, `level_id`, `org_id`, `quantity`, `fulfilled_quantity`, `released_quantity`, `backordered` | (`tenant_id`,`reservation_id`) |
| `inventory_fulfilments` | `id`, `tenant_id`, `reservation_id`, `fulfilment_key`, `fingerprint`, `created_at` | unique (`tenant_id`,`reservation_id`,`fulfilment_key`) |
| `inventory_restocks` | `id`, `tenant_id`, `reference`, `reference_fingerprint`, `order_id?`, `org_id`, `created_at` | unique (`tenant_id`,`reference`) |
| `inventory_sources` | `selling_org_id`, `warehouse_org_id`, `priority`, `active` | unique (`tenant_id`,`selling_org_id`,`warehouse_org_id`) |
| `inventory_settings` | `org_id`, `prefer_nearby`, `allow_split` | unique (`tenant_id`,`org_id`) |
| `inventory_lots` | `id`, `tenant_id`, `level_id`, `lot_code`, `expires_on?` (`date`), `on_hand`, `received_at` | unique (`tenant_id`,`level_id`,`lot_code`); (`tenant_id`,`level_id`,`expires_on`) |
| `inventory_serials` | `id`, `tenant_id`, `variant_id`, `serial`, `org_id?`, `status` (`in_stock`/`sold`/`returned`/`written_off`), `order_id?`, `updated_at` | unique (`tenant_id`,`variant_id`,`serial`) |
| `inventory_transfers` + `inventory_transfer_lines` | `number`, `from_org_id`, `to_org_id`, `status` (`draft`/`in_transit`/`partially_received`/`received`/`closed`/`cancelled`), `shipped_at?`, `reference?` / `variant_id`, `quantity`, `received_quantity`, `unit_cost_minor`, `unit_cost_currency`, `lot_code?` | unique (`tenant_id`,`number`); (`tenant_id`,`to_org_id`,`status`) |
| `inventory_counts` + `inventory_count_lines` | `number`, `org_id`, `status` (`open`/`applied`/`cancelled`), `applied_at?` / `variant_id`, `counted`, `expected_at_count` (`on_hand` lúc đếm dòng đó), `counted_at`, `reason_code?` | (`tenant_id`,`org_id`,`status`) |
| `inventory_suppliers` | `id`, `tenant_id`, `org_id`, `code`, `name`, `name_key`, `tax_code?`, `contact_name?`, `phone?`, `email?`, `address` (cột của `Address`), `payment_terms_days`, `currency`, `active` | unique (`tenant_id`,`code`) |
| `inventory_purchase_orders` + `…_lines` | `number`, `org_id` (kho nhận), `supplier_id`, `status` (`draft`/`ordered`/`partially_received`/`received`/`closed`/`cancelled`), `expected_on?`, `total_minor`, `currency` / `variant_id`, `quantity`, `received_quantity`, `unit_cost_minor`, `unit_cost_currency` | unique (`tenant_id`,`number`); (`tenant_id`,`supplier_id`,`status`) |
| `inventory_receipts` + `…_lines` | phiếu nhập: `number`, `org_id`, `supplier_id?`, `purchase_order_id?`, `reference`, `reference_fingerprint`, `posted_at`, `total_minor`, `currency` / `variant_id`, `quantity`, `unit_cost_minor`, `unit_cost_currency`, `lot_code?`, `expires_on?`, `serials` (JSON) | unique (`tenant_id`,`number`); unique (`tenant_id`,`reference`) |
| `inventory_supplier_returns` + `…_lines` | `number`, `org_id`, `supplier_id`, `receipt_id?`, `reference`, `posted_at` / như phiếu nhập | unique (`tenant_id`,`number`); unique (`tenant_id`,`reference`) |

Số chứng từ (`PO-…`, `PN-…`, `CK-…`, `KK-…`, `TH-…`) cấp bằng `numbering` của bước 36 theo phạm vi
(khách / kho / năm), trong cùng transaction với chứng từ.

### API REST

Dưới `/api/v1/inventory`; envelope, pagination, ETag, `Created` như mọi module; mọi bản ghi theo kho
kiểm quyền theo độ sâu ở nút kho.

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /levels?org_id&variant_id&below_reorder` · `GET /levels/{id}` | `inventory.read` | trả `on_hand`, `available`, `reserved`, `incoming`, giá vốn |
| `PATCH /levels/{id}` | `inventory.settings.manage` | `reorder_point` |
| `GET /movements?org_id&variant_id&kind&from&to` | `inventory.read` | sổ, mới trước |
| `POST /adjustments` | `inventory.adjust` | `{org_id, reference, lines:[{variant_id, quantity, reason_code, note}]}`, `reason_code` bắt buộc |
| `GET /reservations?order_id&status` · `POST /reservations/{id}/release` | `inventory.reservations.read` · `inventory.adjust` | gỡ kẹt bằng tay, có audit |
| `GET/POST /transfers` · `GET /transfers/{id}` · `POST /transfers/{id}/ship` · `POST /transfers/{id}/receive` · `POST /transfers/{id}/close` · `POST /transfers/{id}/cancel` | `inventory.transfer` (nút nguồn khi gửi, nút đích khi nhận) | nhận một phần được |
| `GET/POST /counts` · `PUT /counts/{id}/lines` · `POST /counts/{id}/apply` · `POST /counts/{id}/cancel` · `POST /counts/{id}/import` (CSV) | `inventory.count`; áp dụng cần `inventory.adjust` | lệch có `reason_code` |
| `GET/POST /suppliers` · `PATCH /suppliers/{id}` | `inventory.purchasing.read` · `inventory.purchasing.manage` | |
| `GET/POST /purchase-orders` · `GET/PATCH /purchase-orders/{id}` · `POST /purchase-orders/{id}/order` · `…/close` · `…/cancel` | purchasing | |
| `POST /receipts` · `GET /receipts[/{id}]` | `inventory.receive` | `reference` bắt buộc (`submit_once`); theo PO hay nhập thẳng |
| `POST /supplier-returns` · `GET /supplier-returns[/{id}]` | `inventory.purchasing.manage` | |
| `GET/PUT /sources/{selling_org_id}` · `GET/PUT /settings/{org_id}` | `inventory.settings.manage` | |
| `GET /reports/valuation?org_id&at` | `inventory.read` | tồn × giá vốn bình quân |

### Tool MCP

`inventory.get_levels`, `inventory.list_movements`, `inventory.adjust_stock`,
`inventory.create_transfer`, `inventory.receive_transfer`, `inventory.create_receipt`,
`inventory.list_low_stock` (`read_only`), `inventory.release_reservation` (`destructive`).

### Event

Phát: `inventory.level_changed` (`variant_id`, `org_id`, `on_hand`, `available` — cho `channels`,
báo cáo) · `inventory.movement_recorded` · `inventory.low_stock` (khi `available` đi từ trên xuống ≤
`reorder_point`) · `inventory.out_of_stock` (xuống ≤ 0) · `inventory.reservation_held` ·
`inventory.reservation_confirmed` · `inventory.reservation_released` (lý do) ·
`inventory.reservation_expired` · `inventory.transfer_shipped` · `inventory.transfer_received` ·
`inventory.count_applied` · `inventory.goods_received` · `inventory.supplier_returned` ·
`inventory.lot_expired`.

Nhận bằng `Subscription::optional` (chưa có `orders` thì không lỗi khởi động), mỗi handler gọi đúng
hàm service mà `api` gọi, chịu được nhận lại:

| Event (tên chốt với bước 45) | Việc | Dữ liệu cần trong `after` |
|---|---|---|
| `orders.order_confirmed` | `confirm(order_id)` | `order_id` |
| `orders.order_cancelled` | `release(order_id, Cancelled)` | `order_id` |
| `orders.return_received` | `restock` với `reference = "return:<return_id>"` | `return_id`, `order_id`, `warehouse_org_id`, `lines[{variant_id, quantity, sellable, lot_code?, serials[]}]` |

`orders` vẫn gọi `api` trực tiếp ở bước đặt hàng (giữ chỗ phải đồng bộ); event là đường an toàn khi
lời gọi trực tiếp lỗi sau khi `orders` đã commit.

### Job và lịch

- `inventory.post_receipt_to_ledger` / `inventory.post_supplier_return_to_ledger`: xếp trong cùng
  transaction với phiếu; gọi `Ledger` (bước 36, chữ ký theo đặc tả bước đó) với mã tham chiếu
  `inventory.receipt:<id>` — Nợ hàng tồn kho / Có phải trả nhà cung cấp (đảo lại khi trả hàng). Ledger
  chống trùng theo mã tham chiếu nên chạy lại an toàn.
- Lịch: `inventory.expire_reservations` mỗi phút (mỗi chỗ giữ quá hạn một transaction, đổi trạng thái
  có điều kiện `held → expired` rồi cộng lại `available`); `inventory.expire_lots` mỗi ngày (lô quá
  hạn: biến động `expiry`, trừ `on_hand` và `available`, event `inventory.lot_expired`).
- Task: `inventory.verify_levels [org_id=]` — so `on_hand` với tổng sổ, so `available` với chỗ giữ;
  chỉ báo, không sửa. `Module::checks` có bước kiểm mẫu (tối đa 1.000 dòng, chỉ đọc, snapshot).

### Cấu hình (`AUTOXIDE_INVENTORY_`)

`MAX_HOLD_HOURS` (72), `EXPIRE_BATCH` (200), `MAX_LINES_PER_RESERVATION` (200). Quy tắc theo khách
(nguồn, chia kho, tồn tối thiểu) nằm trong bảng, không ở biến môi trường. Sinh lại `.env.example`.

### Tính năng và hạn mức

`Module::features`: `inventory.multi_warehouse` (> 1 kho), `inventory.transfers`, `inventory.lots`,
`inventory.serials`, `inventory.purchasing`. `Module::limits`: `inventory.warehouses`.

### Permission

`inventory.read`, `inventory.adjust`, `inventory.receive`, `inventory.transfer`, `inventory.count`,
`inventory.reservations.read`, `inventory.settings.manage`, `inventory.purchasing.read`,
`inventory.purchasing.manage`.

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `inventory.insufficient_stock` | 409 | Không đủ hàng cho {lines}. |
| `inventory.no_source` | 409 | Cửa hàng này chưa có kho nào phục vụ. |
| `inventory.reservation_not_found` | 404 | Không có chỗ giữ hàng cho đơn này. |
| `inventory.reservation_expired` | 409 | Chỗ giữ hàng của đơn đã hết hạn hoặc đã trả lại; hãy giữ lại. |
| `inventory.hold_too_long` | 422 | Thời hạn giữ hàng dài quá {max} giờ. |
| `inventory.over_fulfilment` | 409 | Xuất nhiều hơn số đã giữ cho dòng `{line_key}`. |
| `inventory.serials_required` | 422 | Cần đủ {count} số serial cho dòng `{line_key}`. |
| `inventory.serial_unavailable` | 409 | Serial `{serial}` không có trong kho này. |
| `inventory.lot_unavailable` | 409 | Lô `{lot_code}` không đủ hàng hoặc đã hết hạn. |
| `inventory.not_a_warehouse` | 422 | Nút `{org_id}` không phải kho hay cửa hàng. |
| `inventory.level_not_found` | 404 | Không tìm thấy tồn kho. |
| `inventory.transfer_not_found` / `count_not_found` / `supplier_not_found` / `purchase_order_not_found` / `receipt_not_found` | 404 | Không tìm thấy phiếu chuyển kho / phiếu kiểm kê / nhà cung cấp / đơn đặt hàng / phiếu nhập. |
| `inventory.invalid_transition` | 409 | Không chuyển được phiếu từ `{from}` sang `{to}`. |
| `inventory.over_receipt` | 409 | Nhận nhiều hơn số đặt / số đang chuyển. |
| `inventory.reason_required` | 422 | Điều chỉnh tồn cần lý do. |
| `inventory.supplier_code_taken` | 409 | Mã nhà cung cấp `{code}` đã có. |

## Thứ tự làm

1. Khung crate, đăng ký, permission, tính năng / hạn mức.
2. `levels`, `movements`, điều chỉnh có lý do, `verify_levels`; giá vốn bình quân khi nhập → **mục 1**
   (phần giá vốn của **mục 4**).
3. Chỗ giữ: `reserve` / `confirm` / `release` / `fulfil` / `restock`, chọn kho, combo, job hết hạn, test
   "100 người mua cái cuối" → **mục 2**.
4. `api.rs`, event phát ra, handler nhận event của `orders` (test bằng module giả khai các event đó) →
   **mục 5**.
5. Chuyển kho, kiểm kê, cảnh báo tồn thấp, cho bán khi hết → **mục 3**.
6. Lô / hạn dùng (chọn lô hết hạn trước khi xuất), serial → **mục 4**.
7. Nhà cung cấp, đơn đặt hàng, phiếu nhập một phần, trả hàng nhà cung cấp, job ghi sổ cái → **mục 6**.
8. Tài liệu `docs/INVENTORY.md` + `.vi.md`, OpenAPI, `.env.example`, CHANGELOG, ROADMAP.

## Test bắt buộc

Cả SQLite và Postgres, qua `TestDatabase::run` / `TestApp`; `dyn Catalog`, `dyn OrgDirectory`,
`dyn Ledger` bằng bộ giả.

- **`tram_nguoi_mua_cai_cuoi`**: `available = 1`, 100 `reserve` đồng thời (mỗi cái một `order_id`)
  → đúng 1 thành công, 99 `inventory.insufficient_stock`; sau đó `available = 0`, `on_hand = 1`, một
  chỗ giữ `held`. Thêm biến thể: 100 người mua 3 cái khi còn 250 → đúng 83 thành công.
- `giu_nhieu_dong_khong_deadlock`: 50 đơn đồng thời mỗi đơn hai biến thể A, B theo thứ tự ngược nhau →
  không lỗi deadlock lọt ra ngoài, tổng nhất quán.
- `giu_lai_cung_order_id` (16 request đồng thời cùng nội dung → một chỗ giữ, các bên kia `Replayed`;
  khác nội dung → `reference_reused`).
- `xac_nhan_va_het_han_dua_nhau`: `confirm` và job hết hạn chạy cùng lúc → đúng một bên thắng, `available`
  không bị cộng hai lần.
- `tra_cho_hai_lan_chi_cong_mot_lan`, `nhan_lai_event_huy_va_tra_hang` (giao event hai lần).
- `xuat_mot_phan_roi_huy_phan_con_lai`, `xuat_lai_cung_fulfilment_key`.
- `combo_tru_tung_thanh_phan`, `cho_ban_khi_het_cho_available_am`, `san_pham_khong_theo_doi_ton_bo_qua`.
- `chon_kho_cung_tinh_truoc`, `chia_kho_khi_cho_phep`, `khong_nguon_thi_loi`.
- `on_hand_bang_tong_so` (sau chuỗi thao tác ngẫu nhiên có seed cố định: `verify_levels` sạch).
- `chuyen_kho_dang_di_duong_nhan_mot_phan`, `kiem_ke_lech_thanh_dieu_chinh_co_ly_do`,
  `kiem_ke_khi_dang_ban` (bán xen giữa lúc đếm và lúc áp dụng).
- `gia_von_binh_quan_gia_quyen` (số học kiểm bằng tay, làm tròn), `lo_het_han_truoc_xuat_truoc`,
  `serial_khong_ban_hai_lan` (đồng thời).
- `nhap_hang_mot_phan_theo_po`, `phieu_nhap_ghi_so_cai_mot_lan` (job chạy hai lần → ledger giả nhận một
  mã tham chiếu), `tra_hang_nha_cung_cap_dao_but_toan`.
- `canh_bao_ton_thap_chi_khi_vuot_nguong`.
- **Rò rỉ**: bài chung giữa khách; `ro_ri_giua_hai_kho_anh_em` (nhân viên kho A độ sâu `Unit` không
  đọc / điều chỉnh tồn kho B, không thấy chỗ giữ của B).

## Tiêu chí xong

- [ ] Mọi mục của bước 42 trong ROADMAP được phủ (bảng "Thứ tự làm").
- [ ] Không có đường ghi `on_hand` / `available` nào ngoài `rows::adjust`; không có `UPDATE` /
      `DELETE` trên `inventory_movements` (thêm kiểm vào test hay `xtask` nếu rẻ).
- [ ] `docs/INVENTORY.md` + `docs/INVENTORY.vi.md` (mô hình tồn, giữ chỗ, chọn kho, kiểm kê, nhập
      hàng); OpenAPI, `.env.example` sinh lại; CHANGELOG.
- [ ] DECISIONS: mô hình `on_hand` / `available`; chọn kho; cách tính giá vốn và làm tròn; cách kiểm
      kê khi đang bán; nhận event của `orders` qua `Subscription::optional`.
- [ ] `./gate.sh` đầy đủ xanh; đánh dấu ROADMAP; commit; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Thứ tự khoá**: một chỗ giữ nhiều dòng → `adjust` theo thứ tự `level_id` tăng dần; vẫn bọc cả lệnh
  trong `write_with_retry` (Postgres `40P01`, SQLite `BUSY`). Lệnh phải chạy lại được: không gọi mạng,
  không gọi `Catalog` bên trong transaction — lấy biến thể / combo TRƯỚC khi mở transaction.
- **Thử kho kế**: `adjust` trả 0 giữa chừng thì các câu đã chạy trong transaction vẫn còn. Hoặc mở
  savepoint mỗi kho (SeaORM hỗ trợ transaction lồng — tra API), hoặc trả lỗi nội bộ để rollback rồi chọn
  lại kho bằng số liệu đọc mới. Không "cộng bù" bằng tay.
- **Chống trùng**: `reserve` dùng `reference::submit_once` + unique (`tenant_id`, `order_id`);
  `fingerprint` chỉ gồm dòng và kho cố định (không gồm `expires_at` — gọi lại có thể lệch vài giây).
- **Trạng thái đua nhau**: mọi chuyển trạng thái là `UPDATE … WHERE status = <cũ>` rồi xét số dòng;
  chỉ bên đổi được mới cộng / trừ tồn.
- **Âm và tràn**: `quantity` từ API luôn > 0 và ≤ 1.000.000 (kiểm ở service); `Bounds::ANY` khi cho
  bán khi hết vẫn chặn tràn.
- **Giá vốn bình quân** cần đọc rồi ghi: `rows::lock_one` dòng tồn trước; tính bằng `rust_decimal`,
  làm tròn về đơn vị nhỏ nhất (ghi quy tắc); `on_hand ≤ 0` trước khi nhập thì giá vốn = giá nhập.
- **Kiểm kê khi đang bán**: lưu `expected_at_count` lúc đếm từng dòng; lệch = `counted −
  expected_at_count`, áp vào `on_hand` hiện tại — không ghi đè `on_hand` bằng số đếm.
- **Event của `orders` có thể tới trước lời gọi trực tiếp hoặc sau** — cả hai đường dùng cùng hàm, cùng
  điều kiện trạng thái.
- **Lô**: giữ chỗ ở mức dòng tồn, chọn lô lúc xuất; lô hết hạn phải bị trừ khỏi `available` bằng job,
  nếu không sẽ bán hàng hết hạn.
- **SQLite**: một bên ghi — test đồng thời 100 request dựa vào `AUTOXIDE_DATABASE_WAIT_SECONDS`; đặt đủ
  lớn trong test, đừng hạ số request.

## Căn cứ pháp lý

- Phiếu nhập / xuất kho, kiểm kê là chứng từ kế toán: Luật Kế toán 88/2015/QH13 (lưu tối thiểu 10 năm,
  Điều 41) — dữ liệu sổ biến động và phiếu không xoá khi xoá khách hay dữ liệu cá nhân (bước 36). Mẫu
  chứng từ kho theo chế độ kế toán doanh nghiệp hiện hành (Thông tư 200/2014/TT-BTC, Thông tư
  133/2016/TT-BTC và văn bản thay thế) — cần đối chiếu văn bản gốc.
- Không có quy định bắt buộc "hết hạn trước xuất trước"; việc không bán hàng quá hạn sử dụng thuộc luật
  chất lượng sản phẩm, hàng hoá và an toàn thực phẩm — cần người có chuyên môn xác nhận cho từng ngành.

## Agent làm bước này tự quyết

- Savepoint hay chạy lại để thử kho kế — khuyến nghị savepoint nếu SeaORM 2 hỗ trợ trên cả hai DB
  (tra bằng `check-library-api`), không thì đọc `available` trước để chọn kho rồi `adjust` làm trọng tài.
- Quy tắc làm tròn giá vốn — khuyến nghị nửa lên xa số 0 (`MidpointAwayFromZero`) về đơn vị nhỏ nhất.
- Có gộp `inventory.level_changed` (nhiều thay đổi một dòng trong một transaction thành một event) —
  khuyến nghị gộp theo dòng tồn trong mỗi transaction.
- Mặc định `allow_split` — khuyến nghị tắt (một đơn một kho, dễ giao).
- Ghi mọi lựa chọn vào `docs/DECISIONS.md` (skill `record-decision`).

## Bổ sung theo D-200 (thiết kế giao diện 2)

**Tình trạng hàng công khai.** `GET /public/availability?variant_ids&store_id` (công khai) trả cho mỗi biến
thể × cửa hàng một mức: `in_stock`, `low` (kèm `quantity` khi tồn khả dụng dưới
`AUTOXIDE_INVENTORY_PUBLIC_LOW_THRESHOLD`, mặc định 5), `out`; bỏ `store_id` thì trả mọi cửa hàng có
`pickup` (bước 34). Không bao giờ trả số tồn trên ngưỡng. `api` thêm hàm lọc tập biến thể còn hàng ở một
cửa hàng để `catalog` làm bộ lọc "chỉ hàng có sẵn ở cửa hàng X". Cache ngắn (vài giây) và ETag theo phiên
bản sổ biến động. Test: số tồn lớn không lộ; mức đổi ngay sau khi giữ chỗ.

## Bổ sung theo D-201

**Báo khi có hàng lại.** `POST /public/restock-alerts {variant_id, store_id?, contact}` — xác nhận hai
bước như `newsletter`, đồng ý riêng cho mục đích "thông báo có hàng" (bước 37). Nghe
`inventory.level_changed` từ 0 lên trên 0 ở đúng cửa hàng (hay bất kỳ kho bán online khi bỏ
`store_id`), gửi một lần qua `notifications`, rồi đánh dấu đã gửi. Bảng `inventory_restock_alerts` cấp
khách hàng. Test: một đợt có hàng chỉ gửi một lần dù event đến lại.

## Bổ sung theo D-204 (hàng đặt trước)

- Biến thể `preorder` (bước 41): giữ chỗ được dù `available` âm, dòng giữ đánh dấu `preordered = true`
  (bên cạnh `backordered`). Tổng suất đặt trước đang giữ hay đã xác nhận không được vượt `preorder_limit`:
  bảng `inventory_preorder_counters` (`tenant_id`, `variant_id`, `reserved`, `limit` chép từ catalog,
  `updated_at`; pk (`tenant_id`, `variant_id`)), tăng bằng `UPDATE … SET reserved = reserved + q WHERE
  reserved + q <= limit` (không đọc rồi ghi) — 0 dòng thì `inventory.preorder_sold_out` (409). Trả chỗ khi
  huỷ / hết hạn thì trừ lại.
- Hàng về (nhập kho, chuyển kho tới): job `inventory.allocate_preorders` chia tồn mới cho các dòng đặt
  trước **theo thứ tự đặt** (id UUIDv7), phát `inventory.preorder_allocated` từng dòng để `orders` chuyển
  đơn sang chuẩn bị hàng. Nhận lại event nhập kho không chia hai lần (khoá theo id biến động kho).
- Đặt trước không làm `available` công khai thành "còn hàng": mức công khai là `preorder`.
- Test (cả hai database): `dat_truoc_khong_vuot_gioi_han_khi_dong_thoi` (100 người, giới hạn 10),
  `hang_ve_chia_theo_thu_tu_dat`, `huy_dat_truoc_tra_lai_suat`.
