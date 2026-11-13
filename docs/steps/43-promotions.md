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
