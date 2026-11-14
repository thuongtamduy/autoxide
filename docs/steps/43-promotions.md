# Bước 43 — Khuyến mại (`promotions`)

> Lộ trình: `docs/ROADMAP.md` bước 43 và phần "Giai đoạn module thương mại" · Thiết kế:
> `docs/research/business-foundation.md` mục 8 · Quyết định: D-180, D-181, D-184, D-186, D-188
> (`rust_decimal`), D-191

## Mục tiêu

Xong bước này, khách chạy được chương trình khuyến mại phổ biến ở Việt Nam — mã giảm giá, giảm tự động
theo sản phẩm / danh mục / giá trị đơn / nhóm khách, mua X tặng Y, miễn phí vận chuyển, flash sale có
số lượng — với giới hạn lượt chống vượt khi nhiều người dùng cùng lúc. `orders` (45) gọi
`autoxide_promotions::api` để tính giảm giá cho một giỏ (từng dòng kèm lý do người mua đọc được) và giữ
→ xác nhận → trả lượt dùng theo `order_id`. Chương trình vượt hạn mức pháp luật đang hiệu lực bị chặn;
hạn mức là dữ liệu có ngày hiệu lực; xuất được dữ liệu thông báo khuyến mại gửi Sở Công Thương.

## Bắt đầu khi

- Bước 41 xong: `dyn Catalog` (`variants` có `category_ids` kèm tổ tiên, `reference_prices` = giá ngay
  trước khuyến mại), `autoxide_kernel::csv`.
- Từ nền: `TenantContext`, `OrgId`, `Scope`, `dyn OrgDirectory`, `rust_decimal`, `Money::allocate`.
- Song song được với 42, 44.

## Phạm vi

- Crate mới `modules/promotions` (`autoxide-promotions`), đăng ký sau `catalog`.
- Ngoài phạm vi: điểm thưởng, thẻ quà tặng (47); nhóm khách thật (47 — ở đây `customer_groups` là mã
  chữ do bên gọi đưa); giảm giá riêng theo kênh (48); hoa hồng tiếp thị (49); nộp thông báo lên cổng
  dịch vụ công (chỉ xuất dữ liệu, người nộp là con người).

## Thiết kế

### Kiểu và trait

Phác thảo, tra API thật bằng skill `check-library-api`.

```rust
// modules/promotions/src/api.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cart {
    pub org_id: OrgId,                    // cửa hàng bán
    pub currency: Currency,
    pub buyer_id: Option<Uuid>,           // người mua đăng nhập
    pub buyer_key: Option<String>,        // khách vãng lai: băm SĐT / email (do `orders` tính), cho hạn mức mỗi người
    pub customer_groups: Vec<String>,
    pub channel: Option<String>,
    pub lines: Vec<CartLine>,
    pub shipping_fee: Option<Money>,      // phí vận chuyển đã báo (bước 44), để tính miễn phí vận chuyển
    pub codes: Vec<String>,               // mã người mua nhập (tối đa MAX_CODES)
    pub at: DateTime<Utc>,
    pub locale: String,                   // ngôn ngữ của `reason`
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CartLine {
    pub line_key: String,
    pub variant_id: Uuid,
    pub quantity: i64,
    pub unit_price: Money,                // giá `orders` lấy từ `Catalog::prices`
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Evaluation {
    pub lines: Vec<LineDiscount>,         // mọi khoản giảm đã phân về từng dòng
    pub shipping_discount: Money,
    pub gifts: Vec<GiftLine>,             // hàng tặng (mua X tặng Y) — `orders` thêm dòng giá 0
    pub total_discount: Money,
    pub applied: Vec<AppliedPromotion>,
    pub rejected_codes: Vec<RejectedCode>,
    pub fingerprint: String,              // dấu vân tay kết quả, `orders` gửi lại khi giữ
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineDiscount {
    pub line_key: String, pub promotion_id: Uuid, pub amount: Money,
    pub reason: Reason,
    pub capped_by_law: bool,              // đã bị cắt về trần pháp luật
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reason { pub code: String, pub params: BTreeMap<String, String>, pub text: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedPromotion {
    pub promotion_id: Uuid, pub code: Option<String>, pub title: String, pub terms: String,
    pub amount: Money,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedCode { pub code: String, pub reason: Reason }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GiftLine { pub promotion_id: Uuid, pub variant_id: Uuid, pub quantity: i64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReserveRequest {
    pub order_id: Uuid,
    pub cart: Cart,
    pub expected_fingerprint: Option<String>, // khác kết quả tính lại → `promotions.changed`
    pub expires_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedemptionStatus { Held, Confirmed, Released, Expired }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Redemption {
    pub id: Uuid, pub order_id: Uuid, pub status: RedemptionStatus,
    pub evaluation: Evaluation,           // ảnh chụp để chụp vào chứng từ
    pub expires_at: Option<DateTime<Utc>>,
}

#[async_trait::async_trait]
pub trait Promotions: Send + Sync {
    /// Tính giảm giá cho giỏ — không giữ gì, không ghi gì.
    async fn evaluate(&self, tenant: &TenantContext, cart: &Cart) -> Result<Evaluation, Error>;
    /// Tính lại và giữ mọi lượt (chương trình, mã, mỗi người, số lượng flash sale) trong MỘT
    /// transaction: đủ hết hoặc không giữ gì. Gọi lại cùng `order_id` + cùng giỏ → `Replayed`.
    async fn reserve(&self, tenant: &TenantContext, request: &ReserveRequest)
        -> Result<Submitted<Redemption>, Error>;
    /// Giữ → đã dùng. Gọi lại: trả như cũ. Đã hết hạn / trả: `promotions.redemption_expired`.
    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Redemption, Error>;
    /// Trả lượt (huỷ đơn, thanh toán hỏng). Đã xác nhận thì trả khi chương trình cho phép
    /// (`return_usage_on_cancel`). Không có: `Ok(None)`.
    async fn release(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Option<Redemption>, Error>;
}
```

### Cách tính (thứ tự cố định để cùng giỏ luôn ra cùng kết quả)

1. Lấy biến thể từ `Catalog::variants` (sản phẩm, danh mục kèm tổ tiên) và giá tham chiếu từ
   `Catalog::reference_prices` tại `starts_at` của từng chương trình — trước khi tính, ngoài mọi
   transaction.
2. Ứng viên: trạng thái `active`, `starts_at ≤ at < ends_at`, `org_id` của chương trình là cửa hàng
   hay tổ tiên của nó, nhóm khách / kênh khớp; chương trình theo mã chỉ khi mã có trong `codes` và còn
   hiệu lực (mã sai → `rejected_codes` với lý do, không lỗi cả giỏ).
3. Giảm theo dòng (flash sale, giảm theo sản phẩm / danh mục, mua X tặng Y): theo `priority` giảm dần
   rồi `id`; chương trình `exclusive` thắng thì dòng đó không nhận khoản khác; các khoản cộng dồn được
   tính trên phần giá còn lại; không xuống dưới 0.
4. Giảm theo đơn (giá trị đơn tối thiểu, nhóm khách): trên tổng sau bước 3, phân về các dòng đủ điều
   kiện bằng `Money::allocate` theo tỉ lệ tiền dòng (cần cho thuế và đổi trả).
5. Miễn / giảm phí vận chuyển trên `shipping_fee`.
6. **Trần pháp luật**: mỗi dòng, tổng giảm trên một đơn vị ≤ `max_bp` × giá tham chiếu của luật đang
   hiệu lực tại `at` (theo loại khuyến mại và loại chương trình); vượt thì cắt và `capped_by_law = true`.
   Tính bằng `rust_decimal`, làm tròn về đơn vị nhỏ nhất theo quy tắc ghi trong DECISIONS.
7. `reason.text` dịch theo `locale` (key = `reason.code`, vd `promotions.reason.percent_off` với
   `{percent}`, `{min_order}`).

### Bảng dữ liệu

Bảng cấp khách có `tenant_id`; unique / index bắt đầu bằng nó; phần trăm lưu `integer` basis point
(1% = 100); tiền `*_minor` + `*_currency`; JSON trong `text`. Migration có `down`; thêm
`outbox_migration`, `jobs_migration`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `promotions_promotions` | `id`, `tenant_id`, `org_id`, `kind` (`percent_off`/`amount_off`/`fixed_price`/`buy_x_get_y`/`free_shipping`/`flash_sale`), `scope` (`items`/`order`/`shipping`), `trigger` (`automatic`/`code`), `status` (`draft`/`scheduled`/`active`/`paused`/`ended`/`cancelled`), `value_bp?`, `value_minor?`, `value_currency?`, `min_order_minor?`, `min_quantity?`, `max_discount_minor?`, `buy_quantity?`, `get_quantity?`, `starts_at`, `ends_at`, `priority`, `exclusive`, `usage_limit?`, `usage_count`, `per_customer_limit?`, `return_usage_on_cancel`, `program_kind` (`regular`/`national_focus`), `notice_status` (`not_required`/`pending`/`submitted`), `notice_reference?`, `notice_submitted_at?`, `public`, `created_by`, `created_at`, `updated_at`, `activated_at?` | (`tenant_id`,`status`,`starts_at`); (`tenant_id`,`org_id`,`status`) |
| `promotions_translations` | `promotion_id`, `locale`, `title`, `terms` (điều kiện người mua đọc) | unique (`tenant_id`,`promotion_id`,`locale`) |
| `promotions_targets` | `promotion_id`, `target_type` (`product`/`variant`/`category`), `target_id`, `role` (`condition`/`reward`) | (`tenant_id`,`promotion_id`); (`tenant_id`,`target_type`,`target_id`) |
| `promotions_customer_groups` · `promotions_channels` | `promotion_id`, `group_code` / `channel` | unique (`tenant_id`,`promotion_id`,…) |
| `promotions_codes` | `id`, `tenant_id`, `promotion_id`, `code`, `code_key` (chữ hoa, bỏ khoảng trắng), `usage_limit?`, `usage_count`, `created_at` | unique (`tenant_id`,`code_key`); (`tenant_id`,`promotion_id`) |
| `promotions_flash_items` | `promotion_id`, `variant_id`, `sale_price_minor?`, `value_bp?`, `quantity_limit`, `quantity_used` | unique (`tenant_id`,`promotion_id`,`variant_id`) |
| `promotions_customer_usage` | `promotion_id`, `buyer_key` (`user:<id>` hay băm do `orders` đưa), `used` | unique (`tenant_id`,`promotion_id`,`buyer_key`) |
| `promotions_redemptions` | `id`, `tenant_id`, `order_id`, `org_id`, `status`, `fingerprint`, `evaluation` (JSON), `total_discount_minor`, `currency`, `expires_at?`, `created_at`, `updated_at` | unique (`tenant_id`,`order_id`); (`tenant_id`,`status`,`expires_at`) |
| `promotions_redemption_holds` | `redemption_id`, `counter` (`promotion`/`code`/`customer`/`flash`), `counter_id`, `quantity` | (`tenant_id`,`redemption_id`) |
| `promotions_versions` | **chỉ thêm**: `promotion_id`, `version`, `snapshot` (JSON đủ để dựng lại chương trình), `changed_by`, `changed_at` | unique (`tenant_id`,`promotion_id`,`version`) |
| `promotions_legal_limits` | **dữ liệu dùng chung mọi khách (mặt điều khiển)**: `id`, `jurisdiction`, `promotion_kind` (`price_reduction`/`gift_value`/…), `program_kind`, `max_bp`, `effective_from`, `effective_to?`, `legal_basis`, `source_url`, `note`, `pack_version` | unique (`jurisdiction`,`promotion_kind`,`program_kind`,`effective_from`) |
| `promotions_notice_rules` | **dùng chung**: `jurisdiction`, `promotion_kind`, `notice_required`, `days_before`, `authority`, `effective_from`, `effective_to?`, `legal_basis` | như trên |

Giữ lượt: mỗi bộ đếm đổi bằng `rows::adjust` với `Bounds { floor: 0, ceiling: <giới hạn> }` (giới hạn
đọc trong cùng transaction) — `usage_count` của chương trình, của mã, `quantity_used` của flash sale,
`used` của người mua (chèn dòng `used = 0` bằng `ON CONFLICT DO NOTHING` trước). Mỗi lần giữ ghi một dòng
`promotions_redemption_holds`, trả lại đúng các dòng đó.

### API REST

Dưới `/api/v1/promotions`. Chương trình kiểm quyền theo độ sâu ở `org_id`.

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /promotions?status&kind&org_id&q` · `POST /promotions` | `promotions.read` · `promotions.write` | tạo ở `draft` |
| `GET/PATCH/DELETE /promotions/{id}` | read · write | xoá chỉ khi `draft`; sửa sau khi chạy bị giới hạn (dưới) |
| `PUT /promotions/{id}/translations/{locale}` · `PUT /promotions/{id}/targets` · `PUT /promotions/{id}/flash-items` | write | |
| `POST /promotions/{id}/activate` | `promotions.publish` | kiểm trần pháp luật, thông báo, mục tiêu; → `scheduled` / `active` |
| `POST /promotions/{id}/pause` · `…/resume` · `…/end` · `…/cancel` | `promotions.publish` | |
| `POST /promotions/{id}/codes` (sinh lô mã ngẫu nhiên hay một mã đặt tay) · `GET /promotions/{id}/codes` · `GET /promotions/{id}/codes/export` (CSV) | `promotions.codes.manage` | tối đa 10.000 mã / lần, bằng job khi > 500 |
| `GET /promotions/{id}/versions` | `promotions.read` | lưu vết |
| `POST /promotions/{id}/notice-export?format=csv|json` | `promotions.compliance` | dữ liệu thông báo gửi Sở Công Thương |
| `PATCH /promotions/{id}/notice` | `promotions.compliance` | `{status: submitted, reference, submitted_at}` |
| `GET /legal-limits?at` · `GET /notice-rules?at` | `promotions.read` | dữ liệu dùng chung, chỉ đọc |
| `POST /evaluate` | `promotions.read` | thân là `Cart` — cho agent / nhân viên thử; cùng hàm với `api` |
| `GET /redemptions?order_id&status` · `POST /redemptions/{id}/release` | `promotions.read` · `promotions.publish` | gỡ kẹt bằng tay |
| `GET /public/promotions?locale&store_id` | công khai | chương trình `public` đang chạy, tiêu đề + điều kiện (Luật 19/2023); ETag như `content` |

Sửa chương trình đã `active`: chỉ được kéo dài `ends_at`, tăng giới hạn lượt, tạm dừng, kết thúc sớm,
sửa bản dịch không đổi điều kiện; thay đổi khác → `promotions.locked_after_start` (tạo chương trình
mới). Mọi lần đổi sau `activate` thêm một dòng `promotions_versions`.

### Tool MCP

`promotions.list_promotions`, `promotions.get_promotion`, `promotions.create_promotion`,
`promotions.activate_promotion`, `promotions.end_promotion`, `promotions.generate_codes`,
`promotions.evaluate_cart` (`read_only`), `promotions.export_notice` (`read_only`).

### Event

`promotions.promotion_created` · `promotions.promotion_updated` · `promotions.promotion_activated` ·
`promotions.promotion_started` · `promotions.promotion_paused` · `promotions.promotion_ended` ·
`promotions.promotion_cancelled` · `promotions.codes_generated` · `promotions.redemption_confirmed` ·
`promotions.redemption_released` · `promotions.notice_submitted`. Không phát event cho mỗi lần giữ
(quá nhiều); xác nhận và trả có event. Payload không chứa `buyer_key`.

Không nhận event của `orders` ở bước này (`orders` gọi `api`); nếu bước 45 muốn đường an toàn bằng
event thì thêm `Subscription::optional` cho `orders.order_confirmed` / `orders.order_cancelled` như
`inventory`.

### Job và lịch

- `promotions.expire_holds` mỗi phút (đổi `held → expired` có điều kiện, trả bộ đếm theo
  `redemption_holds`); `promotions.tick` mỗi phút (`scheduled → active` khi tới giờ, phát `…_started`;
  `active → ended` khi qua `ends_at`, phát `…_ended`); `promotions.generate_codes` (lô lớn).
- Task: `promotions.load_legal_data pack=vn` — nạp `modules/promotions/data/vn/legal_limits.json` và
  `notice_rules.json` (nguồn, ngày, ghi chú "cần đối chiếu"); chạy lại không nhân đôi.
- `Module::checks`: cảnh báo khi chưa có hạn mức pháp luật nào hiệu lực cho `VN`, hay hạn mức hết
  hiệu lực trong 30 ngày.

### Cấu hình (`AUTOXIDE_PROMOTIONS_`)

`MAX_CODES_PER_CART` (5), `MAX_HOLD_HOURS` (72), `CODE_LENGTH` (10). Sinh lại `.env.example`.

### Tính năng và hạn mức

`Module::features`: `promotions.codes`, `promotions.flash_sale`, `promotions.buy_x_get_y`,
`promotions.customer_groups`. `Module::limits`: `promotions.active_promotions`.

### Permission

`promotions.read`, `promotions.write`, `promotions.publish`, `promotions.codes.manage`,
`promotions.compliance`.

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `promotions.not_found` | 404 | Không tìm thấy chương trình khuyến mại. |
| `promotions.exceeds_legal_limit` | 422 | Mức giảm {value} vượt trần {max} theo {legal_basis} đang hiệu lực. |
| `promotions.notice_required` | 409 | Chương trình cần thông báo cho {authority} trước khi bắt đầu; hãy ghi nhận đã nộp. |
| `promotions.no_legal_data` | 409 | Chưa có dữ liệu hạn mức khuyến mại hiệu lực; chạy `autoxide task promotions.load_legal_data`. |
| `promotions.locked_after_start` | 409 | Chương trình đã chạy chỉ được kéo dài, tăng lượt, tạm dừng hay kết thúc. |
| `promotions.invalid_transition` | 409 | Không chuyển được chương trình từ `{from}` sang `{to}`. |
| `promotions.invalid_window` | 422 | Thời gian kết thúc phải sau thời gian bắt đầu. |
| `promotions.code_taken` | 409 | Mã `{code}` đã có. |
| `promotions.code_invalid` | 422 | Mã `{code}` không tồn tại. |
| `promotions.code_expired` | 422 | Mã `{code}` đã hết hạn hoặc chưa bắt đầu. |
| `promotions.code_not_applicable` | 422 | Giỏ hàng chưa đủ điều kiện dùng mã `{code}`: {condition}. |
| `promotions.usage_exhausted` | 409 | Chương trình `{title}` đã hết lượt. |
| `promotions.customer_limit_reached` | 409 | Bạn đã dùng hết lượt của chương trình `{title}`. |
| `promotions.flash_sold_out` | 409 | Suất giá sốc của `{variant}` đã hết. |
| `promotions.changed` | 409 | Khuyến mại của giỏ đã thay đổi; hãy xem lại trước khi đặt. |
| `promotions.redemption_not_found` | 404 | Không có lượt khuyến mại đang giữ cho đơn này. |
| `promotions.redemption_expired` | 409 | Lượt khuyến mại giữ cho đơn đã hết hạn. |
| `promotions.too_many_codes` | 422 | Mỗi giỏ dùng tối đa {max} mã. |

Thêm các key lý do `promotions.reason.*` (vd `percent_off`: "Giảm {percent}% cho {target}",
`order_min`: "Giảm {amount} cho đơn từ {min_order}", `free_shipping`, `buy_x_get_y`, `flash_sale`,
`capped_by_law`: "Mức giảm đã giới hạn ở {max}% giá trước khuyến mại theo quy định").

## Thứ tự làm

1. Khung crate, permission, tính năng / hạn mức; bảng dữ liệu luật dùng chung + task nạp gói "vn".
2. Chương trình, bản dịch, mục tiêu, nhóm khách, khung thời gian, mã, chuyển trạng thái, phiên bản →
   **mục 1** (phần khai báo).
3. Bộ tính (`evaluate`): mọi loại, cộng dồn, phân về dòng, trần pháp luật, lý do theo ngôn ngữ →
   **mục 1** và **mục 3**.
4. `reserve` / `confirm` / `release`, bộ đếm, flash sale, job hết hạn, test đồng thời → **mục 2**.
5. `api.rs`, REST `/evaluate`, tool MCP, endpoint công khai → **mục 3**.
6. Kiểm tuân thủ khi `activate`, quy tắc thông báo, xuất dữ liệu thông báo, lưu vết → **mục 4**.
7. `docs/PROMOTIONS.md` + `.vi.md`, OpenAPI, `.env.example`, CHANGELOG, ROADMAP.

## Test bắt buộc

Cả SQLite và Postgres; `dyn Catalog`, `dyn OrgDirectory` giả.

- **Đồng thời**: `ma_gioi_han_10_luot_100_nguoi` (100 `reserve` cùng mã → đúng 10 thành công, `usage_count
  = 10`); `flash_sale_5_suat_100_nguoi`; `moi_nguoi_mot_luot_gui_dong_thoi` (cùng người, 10 đơn đồng
  thời → 1); `giu_lai_cung_order_id` (16 request → một lần giữ); `xac_nhan_va_het_han_dua_nhau`;
  `tra_lai_dung_bo_dem_da_giu` (hai lần `release` → cộng lại một lần).
- **Tính**: `cung_gio_cung_ket_qua` (chạy 100 lần, thứ tự dòng xáo trộn → cùng `fingerprint`);
  `exclusive_chan_cong_don`; `giam_theo_don_phan_ve_dong_tong_khop` (tổng phần chia = khoản giảm);
  `khong_giam_xuong_duoi_0`; `mua_x_tang_y`; `mien_phi_van_chuyen`; `nhom_khach_va_kenh`;
  `ma_sai_khong_chan_ca_gio`; `ly_do_theo_ngon_ngu`.
- **Tuân thủ**: `chan_kich_hoat_vuot_tran_50_phan_tram`; `chuong_trinh_tap_trung_quoc_gia_theo_du_lieu`;
  `tran_doi_theo_ngay_hieu_luc` (hai dòng luật, `at` trước / sau ngày đổi); `cat_ve_tran_khi_tinh`
  (giảm theo số tiền trên danh mục vượt 50% giá tham chiếu của một biến thể → `capped_by_law`);
  `can_thong_bao_truoc_khi_chay`; `xuat_du_lieu_thong_bao_chong_csv_injection`;
  `sua_sau_khi_chay_bi_khoa`; `moi_lan_doi_co_phien_ban`.
- **Rò rỉ**: bài chung giữa khách (mã của khách A không dùng được ở khách B, cùng chữ mã);
  `ro_ri_giua_hai_cua_hang_anh_em` (chương trình của cửa hàng A không áp cho giỏ ở cửa hàng B, nhân viên
  B không sửa được).
- `api_promotions_qua_testapp` cho từng hàm của trait.

## Tiêu chí xong

- [ ] Mọi mục của bước 43 trong ROADMAP được phủ.
- [ ] Không có hạn mức pháp luật viết cứng trong code; xoá dữ liệu luật thì `activate` báo
      `promotions.no_legal_data`, không tự cho qua.
- [ ] `docs/PROMOTIONS.md` + `docs/PROMOTIONS.vi.md` (loại khuyến mại, thứ tự tính, giữ lượt, tuân
      thủ, nạp dữ liệu luật); OpenAPI, `.env.example`; CHANGELOG.
- [ ] DECISIONS: thứ tự tính và cộng dồn; quy tắc làm tròn; trần pháp luật là dữ liệu (căn cứ, ngày
      hiệu lực, "cần đối chiếu"); chặn hay cảnh báo khi chưa thông báo.
- [ ] `./gate.sh` đầy đủ xanh; ROADMAP; commit; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Gọi `Catalog` trong transaction**: lấy biến thể và giá tham chiếu trước, rồi mới mở transaction
  giữ lượt; transaction chạy lại được (`write_with_retry`), không gọi ra ngoài.
- **Thứ tự khoá bộ đếm**: nhiều chương trình / mã / flash item trong một lần giữ → `adjust` theo thứ tự
  (`counter`, `counter_id`) cố định.
- **`ceiling` của `adjust` là hằng**: đọc giới hạn trong cùng transaction; giới hạn bị hạ thấp hơn
  `usage_count` hiện có thì `adjust` trả 0 — đúng (hết lượt), không phải lỗi.
- **Chèn dòng mỗi người đua nhau**: `INSERT … ON CONFLICT DO NOTHING` (SeaORM `on_conflict` —
  tra API) rồi `adjust`; không "đọc thấy chưa có thì chèn".
- **Dấu vân tay**: chuẩn hoá giỏ (sắp dòng theo `line_key`, mã theo chữ hoa) trước khi băm; không đưa
  `at` vào — đưa danh sách chương trình đã áp và số tiền.
- **Giá tham chiếu thiếu** (sản phẩm mới, chưa có giá trước khuyến mại): không áp được trần → chặn
  khoản giảm đó (lý do rõ), không coi như không có trần. Ghi quyết định.
- **Làm tròn và phân bổ**: phần trăm trên tổng dòng (không trên đơn giá rồi nhân) để không lệch; phân
  bổ bằng `Money::allocate`; tổng các dòng luôn bằng khoản giảm.
- **Múi giờ**: `starts_at` / `ends_at` lưu UTC; giao diện nhập theo múi giờ của cửa hàng (cài đặt kế
  thừa) — nửa đêm giờ Việt Nam là 17:00 UTC hôm trước.
- **Mã**: so bằng `code_key`; sinh mã ngẫu nhiên bằng `secret::random_token` rồi đổi về bảng chữ không
  nhầm (bỏ `0/O`, `1/I`); unique index quyết định, đụng thì sinh lại.
- **Khách vãng lai**: `buyer_key` là bản băm do `orders` tính — không lưu SĐT / email thô ở đây.

## Căn cứ pháp lý

Đối chiếu ngày 13/11/2026 qua nguồn tổng hợp — **cần đối chiếu văn bản gốc / người có chuyên môn pháp
lý** trước khi chạy thật (D-186):

- Luật Thương mại 36/2005/QH11, mục khuyến mại (Điều 88 trở đi): các hình thức khuyến mại.
- NĐ 81/2018/NĐ-CP về xúc tiến thương mại (thông báo khuyến mại tới Sở Công Thương trước khi thực
  hiện, nội dung hồ sơ), sửa đổi bởi NĐ 128/2024/NĐ-CP; NĐ 239/2026 bỏ các điều về hạn mức trong NĐ
  81/2018 và giao Bộ Công Thương quy định — cần đối chiếu số hiệu và ngày hiệu lực.
- Thông tư 39/2025/TT-BCT: giảm giá tối đa 50% giá hàng hoá, dịch vụ ngay trước thời gian khuyến mại;
  ngoại lệ cho chương trình khuyến mại tập trung quốc gia — dữ liệu `promotions_legal_limits`.
- Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 (từ 01/7/2024): thông tin điều kiện khuyến mại rõ
  ràng → `terms`, `reason`, endpoint công khai.

## Agent làm bước này tự quyết

- Mặc định khi quy tắc thông báo nói "bắt buộc": chặn `activate` (khuyến nghị) hay chỉ cảnh báo.
- Quy tắc làm tròn — khuyến nghị `MidpointAwayFromZero` về đơn vị nhỏ nhất, trên tổng dòng.
- Trả lượt khi huỷ sau xác nhận: mặc định `return_usage_on_cancel = true`.
- Mua X tặng Y: hàng tặng là dòng riêng giá 0 (`gifts`) hay giảm 100% dòng có sẵn — khuyến nghị dòng
  riêng (tồn kho và hoá đơn rõ ràng); chốt với đặc tả bước 45.
- Ghi vào `docs/DECISIONS.md` (skill `record-decision`).

## Bổ sung theo D-200 (thiết kế giao diện 2)

**Giá khi áp mã cho một sản phẩm.** `GET /public/promotions/for-product/{product_id}?variant_id&store_id`
(công khai) trả các mã `public` đang chạy áp được cho sản phẩm: mã, giá sau mã cho số lượng 1, điều kiện
(dùng chung `terms` với `GET /public/promotions`), số lượt còn lại. Giảm tự động (không cần mã) thì
`catalog` hiện giá đã giảm; mã phải nhập thì giá niêm yết giữ nguyên và giao diện hiện thêm dòng "còn X
với mã …" — không trình bày như giá đã giảm sẵn (Luật 19/2023, minh bạch điều kiện). Test: mã hết lượt
không còn trong danh sách; giá sau mã khớp kết quả `evaluate` của giỏ một dòng.

## Bổ sung theo D-201

**Mã dùng được cho một giỏ.** `GET /public/promotions/eligible?cart_id` trả hai danh sách: mã dùng được
(kèm số giảm cho giỏ này) và mã `public` chưa đủ điều kiện kèm `reason` có tham số ("thêm 120.000 ₫",
"giỏ chưa có sản phẩm áp dụng", "chỉ cho đơn đầu tiên"). Website thay ô nhập mã mở sẵn bằng bộ chọn,
ô nhập tự do nằm sau một liên kết. `remaining_uses` chỉ trả khi chương trình có giới hạn lượt, lấy từ số
giữ chỗ thật (không làm tròn, không số giả).

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Giảm tự động và flash sale công khai (B-b7, B-c10).** Phát event khi giá sau giảm tự động của biến thể
  đổi (cho `catalog_storefront_entries`); `GET /public/promotions/flash-sales` trả giờ bắt đầu / kết thúc và
  **suất còn** (số thật, không làm giả độ khan hiếm — Luật 19/2023); điều kiện `first_order_only` (đơn đầu
  của người mua trong khách, hỏi `orders` qua `api`).

## Bổ sung theo D-222 (khuyến mại theo các nền tảng lớn; coupon và voucher)

Phần này **thay** những chỗ nói khác ở trên khi hai bên vênh nhau (vd cờ `exclusive`). Chưa có migration nào
của `promotions` được commit, nên đổi thẳng thiết kế bảng.

### 1. Chương trình, coupon, voucher

- **Chương trình** (`promotions_promotions`) là luật. `trigger` ∈ `automatic` / `code` / `voucher`.
- **Coupon** = dòng `promotions_codes` với `purpose = redeem` của chương trình `code`: ai có mã cũng dùng được,
  kể cả khách vãng lai. Mã chung hay lô mã riêng (đã có). Không có coupon "chỉ một người": việc đó là voucher.
- **Voucher** = dòng `promotions_vouchers` của chương trình `voucher`: một quyền **dùng một lần** trong ví của
  **một tài khoản**, không cần gõ mã. Chỉ người đăng nhập. Không chuyển cho người khác, không đổi ra tiền.
- Mã nhận voucher = `promotions_codes.purpose = claim`: nhập mã → cấp một voucher vào ví (không giảm thẳng).

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `promotions_vouchers` | `id`, `tenant_id`, `promotion_id`, `holder_user_id`, `status` (`available`/`held`/`used`/`expired`/`revoked`), `source` (`claim`/`claim_code`/`grant`/`loyalty`/`purchase_reward`/`birthday`/`referral`/`compensation`/`automation`), `source_ref`, `valid_from`, `expires_at`, `issued_at`, `used_order_id?`, `updated_at` | unique (`tenant_id`,`promotion_id`,`holder_user_id`,`source`,`source_ref`); (`tenant_id`,`holder_user_id`,`status`,`expires_at`); (`tenant_id`,`status`,`expires_at`) |

`promotions_promotions` thêm cho chương trình `voucher`: `voucher_quota?` + `issued_count` (tổng số phát),
`per_user_claim_limit?`, `voucher_valid_days?` (hạn tính từ lúc nhận) hay dùng `ends_at`, `claimable` (hiện nút
"Lưu" công khai), `claim_requires_verified_phone`.

Trait `Promotions` thêm (gọi lại cùng `source` + `source_ref` → trả voucher cũ, không cấp thêm):

```rust
async fn grant_voucher(&self, tenant: &TenantContext, request: &GrantVoucher)
    -> Result<Submitted<Voucher>, Error>;
async fn vouchers_of(&self, tenant: &TenantContext, user_id: Uuid, status: Option<VoucherStatus>)
    -> Result<Vec<Voucher>, Error>;
```

Người gọi: `customers` (sinh nhật, đổi điểm, sau khi mua, giới thiệu bạn — bước 47), `support` (bù lỗi — 53),
`automations` (58). Cấp tay: `POST /promotions/{id}/vouchers` (permission `promotions.vouchers.grant`, lý do bắt
buộc, có audit).

REST cho người mua (dưới `/api/v1/promotions`):

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /me/vouchers?status&store_id` | `promotions.use` | ví voucher, sắp theo hạn gần nhất; `Page` |
| `POST /public/promotions/{id}/claim` | `promotions.use` | bấm "Lưu"; 201; hết suất → `promotions.voucher_quota_exhausted` |
| `POST /me/vouchers/claim-code` `{code}` | `promotions.use` | nhập mã nhận voucher; giới hạn số lần thử |

`Cart` thêm `voucher_ids: Vec<Uuid>`. Voucher phải thuộc `buyer_id`, còn `available` và còn hạn; sai thì vào
`rejected_codes` (giữ tên; mỗi mục mang `code?` hoặc `voucher_id?`) với lý do, không lỗi cả giỏ. Giữ:
`available → held` bằng `UPDATE … WHERE status = 'available'`, xác nhận → `used`, trả → `available` nếu còn hạn,
không thì `expired`. Job `promotions.expire_vouchers` mỗi giờ. `GET /public/promotions/eligible` trả thêm voucher
trong ví dùng được cho giỏ và `best_selection` (tổ hợp mã + voucher lợi nhất theo luật cộng dồn ở mục 2).

Dữ liệu cá nhân: ví nằm trong `PersonalData` export; xoá tài khoản → voucher chưa dùng chuyển `revoked`, voucher
đã dùng giữ cho chứng từ, bỏ liên kết người.

### 2. Lớp và cộng dồn

- `class` ∈ `product` / `order` / `shipping`, suy từ `scope` (`items` → `product`). Thứ tự tính giữ nguyên: lớp
  `product` → `order` → `shipping` → trần pháp luật.
- `exclusivity` ∈ `none` / `class` / `global` (**thay cờ `exclusive`**): `class` = không cộng với chương trình
  khác cùng lớp; `global` = không cộng với bất cứ gì.
- `combines_with` (tập lớp được cộng cùng, vd chương trình lớp `order` cộng được với `shipping` nhưng không với
  `product`). Hai chương trình cộng được khi **cả hai** cho phép (như Shopify).
- `stop_after` (bool): áp chương trình này thì không xét chương trình ưu tiên thấp hơn cùng lớp.
- Chế độ của khách (cài đặt kế thừa `promotions.stacking_mode`): `priority` (mặc định — dễ đoán: theo
  `priority` giảm dần rồi `id`) hay `best_for_buyer`: mỗi lớp so **tập cộng dồn được** với **từng chương trình
  độc quyền đứng riêng**, lấy khoản giảm lớn nhất; bằng nhau → `priority` → `id`. Số phương án ≤ số chương trình
  độc quyền ứng viên, nên tuyến tính, vẫn tất định. Không tìm tổ hợp tối ưu toàn cục.
- Giới hạn: `MAX_CODES_PER_CART` (đã có) và `MAX_VOUCHERS_PER_CLASS` (1) — tính **theo từng bên trả tiền**: trên
  sàn một giỏ dùng được 1 voucher của mỗi người bán + 1 voucher sàn cho mỗi lớp (kiểu Shopee).

### 3. Ai trả tiền

`promotions_funding` (`promotion_id`, `payer_kind` (`platform`/`seller`/`brand`/`payment_partner`),
`payer_org_id?`, `share_bp`; tổng `share_bp` = 10.000, sai → `promotions.invalid_funding`). Mặc định: một dòng
`seller` = nút sở hữu chương trình, 100%. `LineDiscount` thêm `funding: Vec<FundingShare>` (chia bằng
`Money::allocate`, tổng khớp từng đồng). Chương trình của người bán chỉ áp dòng của người bán đó; chương trình
của sàn chia về các đơn con theo tiền dòng. `orders` chụp `funding` vào đơn; `marketplace` quyết toán theo đó
(đặc tả 46); `payment_partner` thành khoản phải thu từ đối tác trong `ledger`.

**Tham gia chương trình của sàn** (flash sale, ngày hội): `promotions_participations` (`promotion_id`,
`seller_org_id`, `status` `pending`/`approved`/`rejected`, `items` JSON: biến thể, giá / mức giảm đề nghị, số
suất, `reviewed_by?`). Người bán đăng ký (`promotions.participate`), sàn duyệt; duyệt xong mới thành
`flash_items`. Phần người bán tự giảm thì `funding` của người bán.

### 4. Ngân sách, lượt theo ngày, khung giờ

- `budget_minor?` + `budget_used_minor`: giữ = cộng khoản giảm của chương trình trong đơn bằng `rows::adjust`
  có trần. Không đủ ngân sách thì **không áp** chương trình (lý do `promotions.reason.budget_exhausted`), không
  áp một phần. Hết ngân sách → event `promotions.budget_exhausted`, chương trình vẫn `active` cho tới khi trả
  lượt làm còn chỗ.
- `daily_usage_limit?`: `promotions_daily_usage` (`promotion_id`, `day` theo múi giờ cửa hàng, `used`; unique)
  — chèn `ON CONFLICT DO NOTHING` rồi `adjust`.
- Khung giờ ("giờ vàng"): `promotions_schedules` (`promotion_id`, `weekdays` (bitmask), `start_minute`,
  `end_minute`) theo múi giờ cửa hàng; không có dòng = cả ngày.

### 5. Loại mới

| `kind` | Nghĩa | Cột / bảng |
|---|---|---|
| `tiered` | Mua nhiều giảm nhiều: 2 món −10%, 3 món −15%; đơn 500k −50k, 1 triệu −120k | `promotions_tiers` (`promotion_id`, `threshold_kind` `quantity`/`amount`, `threshold`, `value_bp?`, `value_minor?`); lấy bậc cao nhất đạt |
| `bundle_price` | Combo đồng giá: chọn N món trong tập mục tiêu giá X ("3 món 199k") | `bundle_quantity`, `bundle_price_minor`; lặp theo `max_applications`; xếp món đắt nhất vào combo trước cho tất định |
| `add_on` | Mua kèm deal sốc: có món điều kiện thì mua món thưởng giá ưu đãi | `role = condition / reward`; `max_add_on_quantity`; giá / mức giảm theo `promotions_flash_items` |
| `gift_choice` | Chọn quà: đạt điều kiện thì chọn `gift_quantity` món trong danh sách | `promotions_gift_options` (`variant_id`, `stock_limit?`, `quantity_used`); `Cart.gift_selections`; chưa chọn → gợi ý, không chặn đặt hàng |
| `buy_x_get_y` (mở rộng) | Mua 2 tính 1, món rẻ nhất miễn phí, mua 2 giảm 50% món thứ hai | `reward_selection` (`cheapest`/`same_item`/`specified`), `reward_value_bp` (10.000 = tặng), `max_applications` |

Hàng tặng và quà giữ tồn qua `orders` → `inventory` như dòng thường giá 0 (đã chốt: dòng riêng).

### 6. Điều kiện mở rộng

`promotions_conditions` (`promotion_id`, `kind`, `operator` `in`/`not_in`/`gte`/`lte`, `values` JSON trong
`text`). Nạp vào bộ nhớ thì biên dịch thành struct, không đọc JSON khi tính. `kind`: `payment_method`,
`carrier`, `ship_to_province`, `membership_tier`, `client` (`web`/`app`/`pos`), `brand`
(thuộc tính), `seller_org`, `attribute`, `new_customer` (chưa có đơn hoàn tất), `returning_customer`,
`min_items`, `exclude_discounted` (bỏ dòng đã có khoản giảm lớp `product` hay đang có giá gạch `compare_at`),
`exclude_targets`. Nhóm / phân khúc khách vẫn qua `promotions_customer_groups` (ID nhóm của bước 47, đã có).
`Cart` thêm `payment_method`, `carrier`, `ship_to_province_code`, `client`, `membership_tier`,
`gift_selections`, `has_compare_at` theo dòng — `orders` điền; mọi trường đi vào `fingerprint`. Đổi phương thức thanh
toán / hãng giao là tính lại.

### 7. Chống lạm dụng

- `per_customer_limit` thêm `per_customer_period` (`lifetime` / `day` / `week` / `month`).
- `Cart.buyer_keys: Vec<String>` thay `buyer_key`: `user:<id>`, băm số điện thoại, băm địa chỉ giao chuẩn hoá
  (`orders` tính; ở đây không lưu bản thô). Giữ lượt cộng **mọi** khoá theo thứ tự cố định; một khoá hết lượt là
  từ chối — một người không lấy thêm lượt bằng tài khoản mới cùng số điện thoại.
- Flash sale: `per_customer_quantity_limit` mỗi biến thể.
- Thử mã: rate limiting của kernel theo người mua / IP cho nhập coupon và mã nhận voucher; lô mã riêng dài ≥ 8 ký
  tự.

### 8. Trả hàng và huỷ

Quy tắc cho `orders` (đặc tả 45) dùng ảnh chụp `Evaluation`:
- Trả một phần: tiền hoàn của dòng = tiền đã trả của dòng × số trả / số mua, chia bằng `Money::allocate` (đơn vị
  cuối nhận phần dư) — tổng hoàn qua nhiều lần trả không vượt tiền đã trả.
- Trả làm đơn rớt dưới ngưỡng (đơn 500k giảm 50k, trả còn 300k): mặc định **không thu lại** khoản giảm (cài đặt
  kế thừa `promotions.clawback_on_return` = `none` / `proportional`).
- Quà / hàng tặng khi trả món điều kiện: `gift_return_policy` của chương trình (`return_gift` /
  `deduct_gift_value` / `keep`), mặc định `return_gift`.
- Huỷ trọn đơn: coupon trả lượt theo `return_usage_on_cancel`; voucher về ví nếu còn hạn. Trả hàng một phần không
  trả voucher.

### 9. Chạy thử, xung đột, lịch, số liệu

- `POST /promotions/{id}/simulate` `{carts: [Cart]}` (≤ 20 giỏ, `promotions.read`): mỗi giỏ kết quả có và không có
  chương trình; chạy được cả khi `draft`. Không ghi gì.
- `GET /promotions/{id}/conflicts`: chương trình trùng mục tiêu và thời gian có `exclusivity` khác `none` →
  cảnh báo; `activate` trả `warnings`, không chặn.
- `GET /promotions/calendar?from&to&org_id`: chương trình theo khoảng ngày (cho lịch của người bán).
- `promotions_stats_daily` (`promotion_id`, `day`, `redemptions`, `discount_minor`, `order_value_minor`,
  `platform_funded_minor`, `seller_funded_minor`; unique) cập nhật trong cùng transaction với `confirm` /
  `release`. `GET /promotions/{id}/stats?from&to`. Số liệu chỉ là số đếm; "doanh thu tăng thêm" để bước 57.

### 10. Nhẹ và nhanh

- `CompiledPromotions` trong bộ nhớ theo (khách, cửa hàng): chương trình `active` / `scheduled` trong 24 giờ
  tới, điều kiện đã biên dịch, chỉ mục `variant_id` / `product_id` / `category_id` / thuộc tính → chương trình.
  Khoá theo `promotions_revisions` (`tenant_id`, `revision`) tăng trong cùng transaction với mọi lần đổi chương
  trình; máy chủ khác nhận event để nạp lại, kèm hạn 60 giây làm lưới an toàn.
- `evaluate` = tra chỉ mục cho dòng trong giỏ → ứng viên → tính; không query database ngoài lần nạp. Mục tiêu:
  giỏ 50 dòng, 500 chương trình đang chạy, p99 < 5 ms phần tính; bench `promotions/benches/evaluate.rs`, số ghi
  `docs/BENCHMARKS.md`.
- `reserve` một transaction: mọi bộ đếm (chương trình, mã, voucher, ngân sách, ngày, mỗi khoá người mua, flash,
  quà) theo thứ tự (`counter`, `counter_id`) cố định.
- Giá sau giảm tự động lớp `product` không có điều kiện giỏ thì ghi sẵn sang `catalog` (event đã có).

### 11. Trần pháp luật và sàn giá

- Cộng **mọi** khoản giảm trên một đơn vị (mọi lớp sản phẩm, mọi bên trả tiền) rồi so trần (D-222, cần đối
  chiếu).
- Sàn giá của `catalog` (`Catalog::price_floors`): `activate` cảnh báo hay chặn theo `action`; khi tính, quy tắc
  `block` cắt khoản giảm như trần pháp luật, lý do `promotions.reason.capped_by_floor`.
- `promotions_notice_rules` có thêm `promotion_kind = purchase_voucher` (voucher cấp sau khi mua — "bán hàng kèm
  phiếu mua hàng", Điều 92 Luật Thương mại; cần đối chiếu).

### Permission, event, lỗi mới

- Permission: `promotions.use` (người mua: ví, lưu, nhập mã nhận — gán sẵn cho vai trò người mua của bước 34),
  `promotions.vouchers.grant`, `promotions.funding.manage` (đặt phần sàn / nhãn hàng trả),
  `promotions.participate` (người bán đăng ký chương trình của sàn).
- Event: `promotions.voucher_issued`, `promotions.voucher_used`, `promotions.budget_exhausted`,
  `promotions.participation_reviewed`. Không phát event cho mỗi voucher hết hạn (job đổi theo lô).
- Lỗi: `promotions.voucher_not_found` (404), `promotions.voucher_not_owned` (403), `promotions.voucher_expired`
  (422), `promotions.voucher_used` (409), `promotions.voucher_quota_exhausted` (409),
  `promotions.claim_limit_reached` (409), `promotions.claim_requires_verified_phone` (403),
  `promotions.budget_exhausted` (409), `promotions.daily_limit_reached` (409), `promotions.outside_schedule`
  (422), `promotions.not_combinable` (422), `promotions.too_many_vouchers` (422), `promotions.invalid_funding`
  (422), `promotions.gift_out_of_stock` (409). Thêm bản dịch và key lý do tương ứng.
- Tool MCP thêm: `promotions.grant_voucher` (cùng service với REST, `promotions.vouchers.grant`; bước 58 gọi),
  `promotions.simulate` (`read_only`), `promotions.list_vouchers_of_user` (`read_only`, cho nhân viên hỗ trợ).
- Tính năng (bước 35): `promotions.vouchers`, `promotions.tiered`, `promotions.bundles`,
  `promotions.budget`. Cấu hình: `MAX_VOUCHERS_PER_CLASS` (1), `MAX_SIMULATE_CARTS` (20),
  `COMPILED_TTL_SECONDS` (60).

### Test bắt buộc thêm (cả SQLite và Postgres)

- Đồng thời: `voucher_quota_10_nguoi_100` (claim), `mot_voucher_hai_don_dong_thoi` (đúng 1),
  `ngan_sach_khong_vuot_khi_100_don`, `luot_theo_ngay_dong_thoi`, `cung_so_dien_thoai_hai_tai_khoan` (khoá chung
  chặn), `qua_het_suat`.
- Cộng dồn: ma trận `exclusivity` × `combines_with` × lớp; `stop_after`; `best_for_buyer_tat_dinh` (100 lần, thứ tự
  dòng xáo trộn → cùng kết quả); `voucher_shop_va_voucher_san_cung_gio`.
- Loại: `bac_thang_lay_bac_cao_nhat`, `combo_dong_gia_lap_lai`, `mua_kem_deal_soc`, `chon_qua`,
  `mua_2_tinh_1_mon_re_nhat`.
- Điều kiện: `theo_phuong_thuc_thanh_toan`, `khung_gio_theo_mui_gio_cua_hang`, `khach_moi`,
  `loai_hang_dang_giam`.
- Tiền: `chia_nguoi_tra_tong_khop`, `tran_cong_ca_voucher_san_va_shop`, `san_gia_cat_khoan_giam`.
- Voucher: `chi_chu_vi_dung_duoc`, `cap_lai_cung_source_ref_khong_nhan_doi`, `huy_don_tra_voucher_con_han`,
  `xoa_tai_khoan_thu_hoi_voucher`, `voucher_trong_xuat_du_lieu_ca_nhan`.
- Hiệu năng: bench ở mục 10 (không là test đỏ / xanh của gate; số đo ghi tài liệu).

## Bổ sung theo đối chiếu thiết kế đợt 2 (D-223…D-227)

> Nguồn: `docs/prompts/ui-phase2-api.md`; màn tương ứng ghi trong `docs/research/ui-traceability.md` (đợt 2).

Theo D-225:
- `ends_at` giữ bắt buộc; "miễn phí giao không thời hạn" là quy tắc phí của bước 44. `promotions_legal_limits` thêm
  `max_days_per_program?`, `max_days_per_year?`; `activate` cộng số ngày các chương trình giảm giá đã chạy cho cùng
  biến thể trong năm (theo `promotions_versions` và mục tiêu) — vượt → `promotions.exceeds_duration_limit` (422).
- `promotions_notices` (`id`, `tenant_id`, `promotion_id`, `authority_code` (tỉnh theo `address`), `status`
  `pending`/`submitted`/`not_required`, `reference?`, `submitted_at?`, `submitted_by?`; unique (`tenant_id`,
  `promotion_id`, `authority_code`)) thay ba cột `notice_status`, `notice_reference`, `notice_submitted_at`. Nơi
  nhận suy từ nơi bán của chương trình (cửa hàng áp dụng, hay "toàn quốc" khi bán online — quy tắc là dữ liệu
  `promotions_notice_rules`). `PATCH /promotions/{id}/notices/{authority_code}`; xuất dữ liệu theo từng nơi.
- `POST /promotions/{id}/activation-check` (`promotions.publish`, không ghi): `checks[]` (`code`, `ok`, `params`) —
  trần giảm, thời hạn, thông báo từng nơi, mục tiêu, ngân sách / funding, xung đột (cảnh báo). `activate` lỗi trả
  cùng danh sách trong `errors[]`.
- Nút "Lưu mã" = `POST /public/promotions/{id}/claim` của voucher (D-222). Không có `/me/saved-promotions`.
- Nhắc flash sale: `POST/DELETE /public/promotions/{id}/reminders` (`promotions.use`); `promotions_reminders`
  (`tenant_id`, `promotion_id`, `user_id`, `channel`, `consent_id`, `remind_at`, `sent_at?`; unique (`tenant_id`,
  `promotion_id`, `user_id`)); job `promotions.send_reminders` mỗi phút gửi qua `notifications`, đánh dấu `sent_at`
  có điều kiện (không gửi hai lần). Tối đa `MAX_REMINDERS_PER_USER` (50) lời nhắc chưa gửi.
- Test: `khong_bat_duoc_khi_thieu_ends_at`, `vuot_so_ngay_giam_gia_trong_nam`, `thong_bao_hai_tinh_mot_tinh_chua_nop`,
  `kiem_truoc_khi_bat_khong_ghi`, `nhac_flash_sale_gui_mot_lan_khi_hai_worker`.
