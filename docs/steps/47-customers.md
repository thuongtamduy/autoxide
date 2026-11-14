# Bước 47 — Khách hàng và giữ chân khách (`customers`)

> Lộ trình: `docs/ROADMAP.md` bước 47 · Thiết kế: `docs/research/business-foundation.md` mục 3 (trụ 2:
> người mua là tài khoản theo khách; trụ 4: nghĩa vụ trong sổ cái), mục 9 · Quyết định: D-185, D-186,
> D-187, D-190, D-194 · Đặc tả liên quan: `docs/steps/45-orders.md`
> Đã gộp các quyết định: D-208…D-218, D-222, D-226 (D-228).
> Nguồn đối chiếu thiết kế: `docs/prompts/ui-trace-roadmap.md` (mã `A-…` / `B-…` trỏ tới
> `docs/research/ui-trace/`) và `docs/prompts/ui-phase2-api.md` (đợt 2, màn ghi trong
> `docs/research/ui-traceability.md`).

## Mục tiêu

Cửa hàng, chuỗi và sàn biết khách của mình và giữ chân họ: hồ sơ bán hàng của từng người mua (lịch sử
mua dựng từ event của `orders`, tổng chi, nhóm và phân khúc theo quy tắc, nhãn, ghi chú của nhân viên);
nhóm khách dùng được cho bảng giá (bước 41) và khuyến mại (bước 43); điểm thưởng và hạng thành viên với
sổ điểm chỉ thêm, điểm có hạn, quy tắc là dữ liệu, điểm chưa dùng là **nghĩa vụ trong sổ cái**; thẻ quà
tặng và số dư cửa hàng **tắt mặc định**, chỉ dùng mua hàng của chính khách (tenant) phát hành, không rút
ra tiền, không chuyển ra ngoài; danh sách yêu thích, sản phẩm vừa xem; nhắc giỏ hàng bỏ dở **chỉ khi
người mua đã đồng ý nhận tin quảng cáo**; giới thiệu bạn bè có thưởng; tặng voucher cho người mua (sinh nhật,
đổi điểm, sau khi mua, giới thiệu bạn) qua `promotions` (D-222). Số điện thoại, email của khách được che khi
nhân viên xem; xem đủ cần permission riêng và có audit (D-226). Người mua tự bật báo khi sản phẩm yêu thích có
hàng lại hay giảm giá thật, với đồng ý riêng, gom lô, không làm phiền (D-233).

Chuỗi bán lẻ ở Việt Nam thường nhận diện khách quen bằng số điện thoại tại quầy, kể cả khi khách không
có tài khoản trên website — hồ sơ vì vậy gắn được với tài khoản người mua **hoặc** với số điện thoại.

## Bắt đầu khi

- Bước 45 (`orders`) đã merge: event `orders.*` (kể cả `orders.cart_abandoned`), `OrderExtensions`
  (`set_buyer_groups`, `add_tender`), `Orders::delivered_lines`; event của `orders` mang `goods_paid_*`
  (cơ sở tính điểm, D-226).
- Sẵn từ trước: 33 (kho secret theo khách — khoá HMAC cho SĐT), 34 (người mua là tài khoản theo khách,
  quyền theo độ sâu), 35 (tính năng / hạn mức), 36 (`Ledger`), 37 (`notifications` với tách tin giao
  dịch / quảng cáo và sổ đồng ý, `compliance` sổ căn cứ xử lý), 43 (`promotions` — cấp voucher bằng
  `Promotions::grant_voucher`, D-222).
- Báo yêu thích (D-233): event giá của `catalog` (41); event tồn của `inventory` (42) không bắt buộc — thiếu
  `inventory` thì chỉ không có báo có hàng lại.
- Song song được với 46, 48, 49.

## Phạm vi

Crate mới `modules/customers` (`autoxide-customers`), đăng ký sau `orders`. Bố cục: `profiles/`,
`segments/` (nhóm, quy tắc), `loyalty/` (điểm, hạng), `stored_value/` (thẻ quà tặng, số dư — sau tính
năng tắt mặc định), `engagement/` (yêu thích, vừa xem, nhắc giỏ), `referrals/`, `tenders.rs` (cài
`orders::api::Tender`), cùng các file chuẩn (`lib.rs`, `api.rs`, `config.rs`, `permissions.rs`,
`events.rs`, `entity/`, `migration/`, `http/`, `tools.rs`, `privacy.rs`, `doctor.rs`, `tests/`).

Sửa ngoài crate: `Cargo.toml`, `crates/app/Cargo.toml`, `crates/app/src/modules.rs`,
`docs/openapi.json`, `.env.example`, `CHANGELOG.md`, `docs/ROADMAP.md`; `modules/catalog` nếu bước 41 chưa có
event và hàm đọc giá hiển thị cho báo yêu thích (D-233).

NGOÀI phạm vi: tài khoản người mua và đăng nhập (bước 34); gửi tin (bước 37 — `customers` chỉ gọi
`Notifier`); bán sỉ B2B (tài khoản công ty, báo giá, công nợ — bước 59 `wholesale`) và chat chăm sóc khách
(bước 61 `conversations`) là module riêng (D-221); ví điện tử, nạp / rút tiền — **không bao giờ** làm (cần
giấy phép, D-187).

## Thiết kế

### Hồ sơ và nhận diện

- Một hồ sơ (`customers_profiles`) cho mỗi người mua trong khách: gắn `user_id` (tài khoản người mua
  của khách) và / hoặc `phone_key` = HMAC-SHA256(SĐT chuẩn hoá E.164, khoá theo khách từ kho secret của
  bước 33). Dùng đúng hàm `contact_key` mà `orders` dùng cho `buyer_key` (API mới của kernel, xem bước 45)
  để ghép được dữ liệu trong cùng khách, không ghép được giữa hai khách.
- Tạo hồ sơ khi: nhận `orders.order_placed` lần đầu của người mua / SĐT; nhân viên đăng ký tại quầy
  (`POST /`); người mua mở trang hồ sơ của mình lần đầu. Gắn tài khoản với hồ sơ có sẵn theo SĐT **chỉ
  khi SĐT của tài khoản đã xác thực** (cờ của bước 37) — không thì ai cũng chiếm được điểm của người khác.
- `org_id` của hồ sơ = nút nơi khách đăng ký / mua lần đầu (cho quyền `Unit` / `Subtree` của nhân viên
  cửa hàng); chuỗi xem toàn khách bằng phạm vi `Tenant`.
- Lịch sử mua: bảng `customers_order_facts` chép tóm tắt đơn từ event (không đọc bảng của `orders`);
  tổng chi = Σ net đơn `completed` − hoàn tiền; cập nhật bằng `rows::adjust`, khoá theo `order_id` để
  event nhận lại không cộng hai lần.
- **Net của đơn** (`customers_order_facts.net_*`, cũng là cơ sở tính điểm — D-226): tiền người mua thực trả cho
  hàng hoá, sau mọi khoản giảm (kể cả coupon, voucher), gồm thuế (giá bán lẻ gồm thuế), không gồm phí giao, không
  gồm phần trả bằng điểm / thẻ quà tặng; trừ phần hoàn khi trả hàng. `orders` gửi `goods_paid_*` trong event.
  Định nghĩa cố định, ghi trong tài liệu; không là tuỳ chọn của khách.
- **Che liên hệ (D-226).** Danh sách, chi tiết, tool MCP và xuất danh sách trả `phone_masked`, `email_masked`
  (`09•• ••• 412`, `n•••@gmail.com`). Xem đủ: `GET /{id}/contact` (`customers.contact.reveal`), ghi audit mỗi
  lần (người xem, hồ sơ, lý do tuỳ chọn); tool `customers.reveal_contact` cùng service. Xuất danh sách có cột
  liên hệ đầy đủ cũng cần permission này. Người mua xem dữ liệu của chính mình (`/me`) không bị che.

### Nhóm và phân khúc

Nhóm `manual` (nhân viên thêm) hay `rule` (quy tắc là dữ liệu JSON có kiểm: `total_spent_min`,
`orders_count_min`, `last_order_within_days`, `tier_in`, `tags_any`, `org_in`). Job đánh giá lại khi
hồ sơ đổi (enqueue, `unique_key` = hồ sơ + khe 1 phút — khoá job của kernel sống tới khi job xong bị
dọn, khoá cố định sẽ chặn lần sau) và toàn bộ hằng đêm. `BuyerGroups::groups_of` (cắm vào
`orders`) trả ID nhóm; `orders` chuyển cho `catalog` (bảng giá theo nhóm) và `promotions` (giảm theo
nhóm, `Cart.customer_groups`). ID nhóm là UUID mờ với `catalog` / `promotions` — không bảng nào join chéo.
Hạng thành viên cũng vào giỏ: `BuyerGroups::tier_of(tenant, user_id) -> Option<String>` (cài mặc định trả
`None`); `orders` điền `Cart.membership_tier` (D-222).

### Điểm thưởng và hạng

- Điểm thưởng là cách thanh toán (`Tender` loại `points`), không phải khuyến mại; voucher đổi từ điểm là
  khuyến mại (D-222).
- Chương trình (`customers_loyalty_rules`, dữ liệu có ngày hiệu lực): số điểm cho mỗi `earn_per_minor`
  tiền net (định nghĩa ở "Hồ sơ và nhận diện", D-226), hệ số theo danh mục, điểm tối thiểu để đổi,
  **giá trị một điểm** (`point_value_minor`), hạn dùng điểm (tháng), trần đổi theo % đơn.
- Tích điểm khi nhận `orders.order_completed` (đã qua hạn đổi trả — ít phải đảo); mỗi lần tích là một
  **lô** (`customers_points_lots`) có hạn riêng. Đổi điểm tiêu lô cũ nhất trước (khoá các lô theo `id`
  bằng `rows::lock`). Sổ điểm (`customers_points_entries`) chỉ thêm; số dư ở
  `customers_points_balances`, đổi bằng `rows::adjust` với `Bounds::NON_NEGATIVE`.
- Đổi điểm khi đặt hàng là một `Tender` loại `points` cắm vào `orders`: `hold(ref "orders:<id>")` đặt
  lô sang giữ, có hạn → xác nhận khi nghe `orders.order_confirmed` → trả khi nghe
  `orders.order_cancelled` hay hết hạn (job). Hoàn tiền đơn có trả bằng điểm (`orders.refund_completed`
  kèm phần của tender) → trả điểm thành lô mới, hạn = max(hạn cũ, hôm nay + 30 ngày).
- Hạng (`customers_tiers`): ngưỡng chi tiêu trong 12 tháng trượt, hệ số tích, nhóm khách gắn với hạng;
  job hằng ngày xét lên / xuống hạng, có event.
- Sổ cái (`Ledger::post`, `reference` = `customers:points:<entry_id>`, qua job enqueue cùng transaction):

| Việc | Nợ | Có |
|---|---|---|
| Tích điểm | `loyalty_expense` | `loyalty_liability` (điểm × giá trị điểm tại lúc tích) |
| Đổi điểm (xác nhận) | `loyalty_liability` | `buyer_prepayments` |
| Hết hạn | `loyalty_liability` | `loyalty_breakage` |
| Trả điểm khi hoàn | đảo bút toán đổi | |
| Đảo tích (hoàn sau hoàn tất) | `loyalty_liability` | `loyalty_expense` (nếu điểm đã tiêu hết: số dư âm có trần 0 — ghi phần không thu hồi được vào `loyalty_expense`, không làm số dư âm) |

### Voucher cho người mua (D-222)

Cấp qua `Promotions::grant_voucher` (không tự tạo mã); gọi lại cùng `source_ref` không cấp thêm:
- Sinh nhật: job hằng ngày theo múi giờ của khách, `source_ref = birthday:<năm>`.
- Đổi điểm lấy voucher: trừ điểm và cấp voucher cùng `source_ref = points:<id bút toán điểm>`; cấp lỗi thì hoàn
  điểm bằng bút toán đảo.
- Sau khi mua: nghe `orders.order_completed`, `source_ref = purchase:<order_id>`.
- Giới thiệu bạn: `source_ref = referral:<id>` (mục "Giới thiệu bạn" dưới).

### Thẻ quà tặng và số dư cửa hàng (TẮT mặc định)

Hai lớp tắt: biến `AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED` (mặc định `false`, người vận hành chỉ bật
sau khi đối chiếu NĐ 52/2024 với người có chuyên môn) **và** tính năng theo khách `customers.gift_cards`
/ `customers.store_credit` (bước 35). Tắt → endpoint trả 403 `customers.stored_value_disabled`, tender
không được cắm vào `orders`, job không chạy. `autoxide doctor` cảnh báo khi đã bật (`customers.stored_value`).

Luật cứng khi bật (giữ tính chất "phương tiện thanh toán khép kín" — cần đối chiếu): chỉ dùng mua hàng
của chính khách (tenant) phát hành — có thể giới hạn thêm theo nhánh (`org_scope`); **không** có
endpoint rút tiền, chuyển số dư sang người khác, đổi ra tiền mặt; hoàn tiền đơn trả bằng thẻ / số dư
trả lại vào thẻ / số dư; số dư cửa hàng chỉ phát sinh từ hoàn tiền mà người mua chọn nhận dạng này hay
khoản bồi thường của nhân viên (có `reference`, lý do), không nạp bằng tiền; hạn mức giá trị một thẻ là
dữ liệu (`max_face_value`). Mã thẻ: 16 ký tự ngẫu nhiên không nhập nhằng, chỉ lưu băm + 4 ký tự cuối;
tra cứu mã có giới hạn số lần (`throttle` của kernel) chống dò.

Sổ cái: phát hành bán qua `orders` (biến thể khai trong `customers_gift_card_products`, phát hành khi
`orders.order_confirmed`): Nợ `buyer_prepayments` / Có `gift_card_liability`; dùng: Nợ
`gift_card_liability` / Có `buyer_prepayments`; số dư cửa hàng từ hoàn tiền: Nợ `refund_payable` / Có
`store_credit_liability`. Thuế GTGT khi bán thẻ / khi dùng thẻ — **cần đối chiếu**, khuyến nghị coi bán
thẻ là nhận trước (không xuất hoá đơn bán hàng), xuất hoá đơn khi dùng thẻ mua hàng.

### Yêu thích, vừa xem, nhắc giỏ bỏ dở, giới thiệu bạn

- Yêu thích: theo sản phẩm hay biến thể, tối đa `WISHLIST_MAX`. Đăng nhập thì gộp danh sách yêu thích lưu ở
  trình duyệt vào tài khoản, bỏ trùng (`POST /me/wishlist/merge {items}`, D-208…D-218, B-c13).
- Vừa xem: tối đa `RECENT_VIEWS_MAX` mỗi hồ sơ, giữ `RECENT_VIEWS_DAYS` ngày (job dọn), chỉ khi người mua
  đăng nhập (vãng lai: client tự giữ).
- Nhắc giỏ: nghe `orders.cart_abandoned` → hồ sơ có đồng ý nhận tin quảng cáo theo sổ đồng ý của bước
  37? Không → ghi `skipped_no_consent`, dừng. Có → tạo lịch theo `customers_reminder_rules` (dữ liệu: các
  mốc trễ, mẫu tin, kênh) → job `customers.send_cart_reminder` (chạy lúc `run_at`) kiểm lại đồng ý và
  giỏ còn bỏ dở (`orders.order_placed` có `cart_id` đó → huỷ lịch) → `Notifier::send` loại
  **Marketing** (notifications áp khung giờ, từ chối là dừng ngay). Mỗi tin có link huỷ nhận.
- Giới thiệu bạn: mỗi hồ sơ một mã; người được giới thiệu nhập mã trong `REFERRAL_APPLY_DAYS` ngày sau
  khi tạo tài khoản và trước đơn đầu tiên; đủ điều kiện khi đơn đầu `completed` với giá trị tối thiểu;
  thưởng (điểm, hay voucher qua `Promotions::grant_voucher` với `source_ref = referral:<id>` — D-222, hay số
  dư nếu đã bật)
  theo `customers_referral_rules` có ngày hiệu lực. Chống gian lận: cùng `phone_key` / email / thiết
  bị với người giới thiệu → `rejected`.

### Báo khi sản phẩm yêu thích có hàng lại hay giảm giá (D-233)

Người mua được báo khi sản phẩm trong danh sách yêu thích có hàng lại hay giảm giá thật — chỉ khi chính họ bật,
không báo giảm giá ảo, không làm phiền.

- **Tự bật, mặc định tắt.** Hai loại bật riêng: `back_in_stock`, `price_drop` (`customers_wishlist_alert_settings`).
  Bật cần đồng ý theo mục đích riêng `customers.wishlist_alerts` trong sổ đồng ý của bước 37 (`category`
  `marketing`, `legal_basis` `consent`; khai vào sổ mục đích như `inventory` khai mục đích báo có hàng của
  D-201). `PUT /me/wishlist/alerts` gọi `ConsentLedger::grant` (subject `User`, `source_ref` =
  `customers.wishlist_alerts:<profile_id>:<lần>`) **trước** transaction của mình; tắt cả hai loại gọi
  `withdraw`. Nghe `compliance.consent_withdrawn` có `purpose` này (lấy người qua `ConsentLedger::get`) → tắt
  cả hai loại, xoá hàng đợi ngay. Bật mà thiếu `consent` → `customers.wishlist_alert_consent_required`; bật
  `back_in_stock` khi không có `inventory` → `customers.wishlist_alert_unavailable`. Tính năng theo khách
  `customers.wishlist_alerts` (bước 35).
- **Gửi** qua `Notifier::notify` (bước 37) loại `customers.wishlist_alert`: `Category::Marketing`, `purpose =
  "customers.wishlist_alerts"`, kênh email, ZNS, web push, mobile push, in-app. `notifications` kiểm lại đồng ý
  ngay trước khi gửi, áp khung giờ, nhãn quảng cáo, link huỷ nhận (bước 37).
- **Giá mốc trên dòng yêu thích.** `customers_wishlist_items` có `baseline_price_*` (giá hiển thị lúc thêm vào
  yêu thích; chưa có giá thì đặt ở lần đầu thấy giá), `back_in_stock_alerted_at`, `price_drop_alerted_at`.
  - **Giá hiển thị** = giá thấp nhất người mua thấy công khai ở nút gốc của khách: giá niêm yết của bảng giá mặc
    định, hay giá sau giảm tự động nếu đang có. Không tính giá theo nhóm khách, mã giảm giá, và **không dùng giá
    gạch `compare_at`** người bán tự đặt. Dòng theo sản phẩm lấy giá thấp nhất của các biến thể; dòng theo
    biến thể lấy giá của biến thể đó.
  - Giá tăng thì mốc giữ nguyên: tăng rồi giảm về mốc không phải giảm giá. Mốc chỉ đổi khi đã báo (= giá đã
    báo). Khác tiền tệ với mốc → đặt lại mốc, không báo.
- **Nghe event** (event chỉ là tín hiệu; handler đọc lại giá / tồn hiện tại theo lô, không tin số trong event
  đến trễ hay đến lại):
  - Có hàng lại: `inventory.level_changed` (`Subscription::optional`) với `before.available ≤ 0` và
    `after.available > 0` ở kho bán online (như D-201 khi không nêu cửa hàng) → dòng theo biến thể đó, và dòng
    theo sản phẩm của biến thể (`Catalog::variants` đổi biến thể ra sản phẩm).
  - Giảm giá: `catalog.price_changed`, `catalog.price_effective` (giá niêm yết) và event của `catalog` khi giá
    sau giảm tự động trên `catalog_storefront_entries` đổi. Giá hiển thị đọc qua `catalog::api`. Bước 41 chưa
    có event giá sau giảm hay hàm đọc giá hiển thị cho nhiều ID thì bước này thêm vào `catalog` (tên do
    `catalog` chốt — README, "Event giữa module thương mại").
  - Điều kiện giảm giá: `giá_mới × 10000 ≤ mốc × (10000 − WISHLIST_PRICE_DROP_MIN_BP)` (số nguyên, `i128`;
    mặc định 500 = giảm ≥ 5%).
  - Đủ điều kiện, loại đó đang bật, và `<loại>_alerted_at` trống hay cũ hơn 7 ngày → ghi (upsert) một dòng
    `customers_wishlist_alert_queue`. Không gửi trong handler.
- **Gom lô** — job `customers.send_wishlist_alerts` hằng giờ (`unique_key` có khe giờ):
  - Chỉ gom trong khung giờ quảng cáo: giao các khung của `notifications.send_windows` (đọc qua
    `dyn ComplianceRules`; kênh `null` không giới hạn); không kênh nào có khung → 07:00–22:00 giờ của khách.
    Ngoài khung thì hàng đợi chờ lô sau; `notifications` vẫn kiểm khung giờ lúc gửi (lớp thứ hai).
  - Mỗi hồ sơ một tin, tối đa 10 sản phẩm; tối đa `WISHLIST_ALERTS_PER_DAY` tin mỗi người mỗi ngày (theo ngày
    của khách, đếm ở `customers_wishlist_alert_batches`). Phần còn lại chờ lô sau.
  - Trước khi gửi kiểm lại: loại còn bật, `ConsentLedger::is_granted`, còn hàng (`Inventory::availability`, có
    `inventory`), giá hiện tại vẫn đạt ngưỡng so với mốc, sản phẩm còn bán. Không đạt → bỏ dòng hàng đợi.
  - Transaction ngắn: chèn dòng lô (`pending`), cập nhật **có điều kiện** `<loại>_alerted_at = now`
    (`WHERE <loại>_alerted_at IS NULL OR < now − 7 ngày`), đặt `baseline_price_*` = giá đã báo, xoá hàng đợi. Dòng
    nào không cập nhật được (worker khác đã báo) thì bỏ khỏi tin. Sau commit gọi `Notifier::notify` với
    `dedupe_key = customers.wishlist_alert:<batch_id>` (không gọi trong transaction ghi); lỗi thì retry gọi lại
    cùng `dedupe_key`, xong đặt lô `sent` và phát `customers.wishlist_alert_sent`.
- **Chống phiền:** mỗi người mỗi sản phẩm tối đa 1 lần / 7 ngày cho mỗi loại (cố định, ghi trong tài liệu); gom
  nhiều sản phẩm vào một tin; trần tin mỗi ngày; mỗi tin có link tắt báo.

### Kiểu và trait (phác thảo — tra API thật bằng skill `check-library-api`)

```rust
// customers::api
pub struct CustomerId(pub Uuid);
pub struct CustomerSummary { pub id: CustomerId, pub user_id: Option<Uuid>, pub org_id: OrgId,
    pub tier: Option<String>, pub group_ids: Vec<Uuid>, pub total_spent: Money, pub orders_count: i64 } // #[non_exhaustive]
#[async_trait] pub trait Customers: Send + Sync {
    async fn find_by_user(&self, tenant: TenantId, user_id: Uuid) -> Result<Option<CustomerSummary>, Error>;
    async fn groups_of_user(&self, tenant: TenantId, user_id: Uuid) -> Result<Vec<Uuid>, Error>;
}
// `CustomerSummary` và mọi danh sách cho nhân viên mang `phone_masked`, `email_masked`, không mang SĐT /
// email đầy đủ (D-226).
// Cài trong customers, cắm vào orders lúc start:
//   orders_ext.set_buyer_groups(Arc::new(GroupsAdapter(..)))?;   // groups_of + tier_of (D-222)
//   orders_ext.add_tender(Arc::new(PointsTender(..)))?;   // luôn
//   if stored_value_enabled { add_tender(GiftCardTender), add_tender(StoreCreditTender) }
```

Giả định: `orders::api::{OrderExtensions, Tender, BuyerGroups}` (bước 45; `BuyerGroups` có
`tier_of(tenant, user_id) -> Option<String>` với cài mặc định trả `None` — D-222);
`notifications::api::Notifier::notify` (bước 37) và kiểm đồng ý bằng `ConsentLedger` của `compliance` (bước 37 — đặc
tả bên cung cấp thắng; D-233 sửa tên cũ `send` / `has_marketing_consent`);
`promotions::api::Promotions::grant_voucher(tenant, GrantVoucher { promotion_id, user_id, source, source_ref })`
(bước 43, D-222);
`ledger::api::Ledger::post` (bước 36); event tạo tài khoản người mua của bước 34 (để tạo hồ sơ sớm — không
bắt buộc). Báo yêu thích (D-233): `compliance::api::{ConsentLedger, ComplianceRules}` (bước 37),
`notifications::api::Notifier::notify` (bước 37), `catalog::api::Catalog` (bước 41), `dyn Inventory` qua
`StartContext::optional` (bước 42).

### Bảng dữ liệu

Quy ước như bước 45 (`tenant_id` đầu mọi khoá; UUIDv7; UTC; `Money` hai cột; JSON trong `text`;
migration có `down`; `outbox_migration`, `jobs_migration`).

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `customers_profiles` | `id`, `tenant_id`, `org_id`, `user_id` NULL, `phone_key` NULL, `phone` NULL, `email_lower` NULL, `full_name` NULL, `full_name_search` (chữ thường không dấu — `autoxide_kernel::search`), `birthday` NULL, `gender` NULL, `tags` text, `tier_code` NULL, `total_spent_*`, `orders_count` BIGINT, `first_order_at` NULL, `last_order_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `user_id`); unique (`tenant_id`, `phone_key`); index (`tenant_id`, `org_id`, `id`), (`tenant_id`, `full_name_search`), (`tenant_id`, `email_lower`) |
| `customers_notes` | `id`, `tenant_id`, `profile_id`, `author_id`, `body`, `created_at` | index (`tenant_id`, `profile_id`, `id`) |
| `customers_order_facts` | `tenant_id`, `order_id`, `profile_id`, `org_id`, `channel`, `status`, `net_*` (tiền thực trả cho hàng hoá, từ `goods_paid_*`, D-226), `refunded_*`, `placed_at`, `completed_at` NULL, `updated_at` | pk (`tenant_id`, `order_id`); index (`tenant_id`, `profile_id`, `placed_at`) |
| `customers_groups` | `id`, `tenant_id`, `org_id` NULL (nhóm của một nhánh), `name`, `kind` (`manual/rule`), `rule` text NULL, `created_at`, `updated_at` | unique (`tenant_id`, `name`) |
| `customers_group_members` | `tenant_id`, `group_id`, `profile_id`, `source` (`manual/rule`), `added_at` | pk (`tenant_id`, `group_id`, `profile_id`); index (`tenant_id`, `profile_id`) |
| `customers_loyalty_rules` | `id`, `tenant_id`, `org_id` NULL, `earn_per_minor` BIGINT, `points_per_unit` INT, `category_multipliers` text, `point_value_minor` BIGINT, `currency`, `points_valid_months` INT, `min_redeem_points` BIGINT, `max_redeem_ratio_bp` INT, `effective_from`, `effective_to` NULL | index (`tenant_id`, `org_id`, `effective_from`) — chỉ thêm |
| `customers_tiers` | `id`, `tenant_id`, `code`, `name`, `threshold_*` (chi 12 tháng), `earn_multiplier_bp`, `group_id` NULL, `rank` INT, `effective_from`, `effective_to` NULL | unique (`tenant_id`, `code`, `effective_from`) |
| `customers_points_balances` | `tenant_id`, `profile_id`, `balance` BIGINT, `held` BIGINT, `updated_at` | pk (`tenant_id`, `profile_id`) |
| `customers_points_lots` | `id`, `tenant_id`, `profile_id`, `earned` BIGINT, `remaining` BIGINT, `held` BIGINT, `value_minor_per_point` BIGINT, `expires_at`, `source_ref`, `created_at` | unique (`tenant_id`, `source_ref`); index (`tenant_id`, `profile_id`, `expires_at`, `id`), (`tenant_id`, `expires_at`) |
| `customers_points_entries` | `id`, `tenant_id`, `profile_id`, `lot_id` NULL, `kind` (`earn/hold/redeem/release/expire/reverse/adjust/restore`), `points` BIGINT (âm / dương), `reference`, `reason` NULL, `actor_id` NULL, `created_at` | unique (`tenant_id`, `reference`, `kind`, `lot_id`) (lot NULL → cột `lot_key`); index (`tenant_id`, `profile_id`, `id`) — chỉ thêm |
| `customers_point_holds` | `tenant_id`, `reference`, `profile_id`, `points`, `lots` text, `status` (`held/confirmed/released`), `expires_at`, `created_at` | pk (`tenant_id`, `reference`); index (`tenant_id`, `status`, `expires_at`) |
| `customers_gift_cards` | `id`, `tenant_id`, `org_scope` NULL, `code_hash`, `code_last4`, `initial_*`, `balance_*`, `status` (`active/void/expired`), `expires_at` NULL, `owner_profile_id` NULL, `issued_reference`, `issued_by` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `code_hash`); unique (`tenant_id`, `issued_reference`); index (`tenant_id`, `owner_profile_id`) |
| `customers_store_credits` | `tenant_id`, `profile_id`, `balance_*`, `updated_at` | pk (`tenant_id`, `profile_id`) |
| `customers_stored_value_entries` | `id`, `tenant_id`, `instrument` (`gift_card/store_credit`), `instrument_id`, `kind` (`issue/hold/redeem/release/refund/void/expire/adjust`), `amount_*`, `reference`, `reason` NULL, `actor_id` NULL, `created_at` | unique (`tenant_id`, `instrument`, `reference`, `kind`); index (`tenant_id`, `instrument`, `instrument_id`, `id`) — chỉ thêm |
| `customers_stored_value_holds` | `tenant_id`, `reference`, `instrument`, `instrument_id`, `amount_*`, `status`, `expires_at` | pk (`tenant_id`, `reference`, `instrument`) |
| `customers_gift_card_products` | `tenant_id`, `variant_id`, `face_value_*`, `valid_months` NULL | pk (`tenant_id`, `variant_id`) |
| `customers_wishlist_items` | `tenant_id`, `profile_id`, `product_id`, `variant_id` NULL, `variant_key` (`variant_id` hay `''`), `added_at`; báo yêu thích (D-233): `baseline_price_*` NULL, `baseline_at` NULL, `back_in_stock_alerted_at` NULL, `price_drop_alerted_at` NULL | pk (`tenant_id`, `profile_id`, `product_id`, `variant_key`); index (`tenant_id`, `product_id`, `variant_key`) (tra theo event, D-233) |
| `customers_wishlist_alert_settings` (D-233) | `tenant_id`, `profile_id`, `back_in_stock` BOOL, `price_drop` BOOL, `updated_at` | pk (`tenant_id`, `profile_id`) — không có dòng = tắt cả hai |
| `customers_wishlist_alert_queue` (D-233) | `tenant_id`, `profile_id`, `product_id`, `variant_key`, `kind` (`back_in_stock/price_drop`), `price_*` NULL (giá lúc phát hiện), `detected_at` | pk (`tenant_id`, `profile_id`, `product_id`, `variant_key`, `kind`); index (`tenant_id`, `detected_at`) |
| `customers_wishlist_alert_batches` (D-233) | `id`, `tenant_id`, `profile_id`, `slot` (giờ UTC của lô), `local_day` (ngày theo múi giờ của khách), `items` text (sản phẩm, loại, giá mốc / giá mới), `status` (`pending/sent`), `created_at`, `sent_at` NULL | unique (`tenant_id`, `profile_id`, `slot`); index (`tenant_id`, `profile_id`, `local_day`), (`tenant_id`, `status`, `created_at`) |
| `customers_recent_views` | `tenant_id`, `profile_id`, `product_id`, `viewed_at` | pk (`tenant_id`, `profile_id`, `product_id`); index (`tenant_id`, `viewed_at`) |
| `customers_reminder_rules` | `id`, `tenant_id`, `org_id` NULL, `delays_minutes` text (vd `[60, 1440]`), `template`, `channel`, `enabled`, `effective_from`, `effective_to` NULL | index (`tenant_id`, `org_id`, `effective_from`) |
| `customers_cart_reminders` | `id`, `tenant_id`, `cart_id`, `profile_id`, `step` INT, `run_at`, `status` (`scheduled/sent/cancelled/skipped_no_consent`), `created_at`, `updated_at` | unique (`tenant_id`, `cart_id`, `step`); index (`tenant_id`, `status`, `run_at`) |
| `customers_referral_rules` | `id`, `tenant_id`, `referrer_reward` text, `referee_reward` text, `min_first_order_*`, `effective_from`, `effective_to` NULL | index (`tenant_id`, `effective_from`) |
| `customers_referral_codes` | `tenant_id`, `profile_id`, `code`, `created_at` | pk (`tenant_id`, `profile_id`); unique (`tenant_id`, `code`) |
| `customers_referrals` | `id`, `tenant_id`, `referrer_profile_id`, `referee_profile_id`, `status` (`pending/qualified/rewarded/rejected`), `reason` NULL, `qualifying_order_id` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `referee_profile_id`); index (`tenant_id`, `referrer_profile_id`, `id`) |

### API REST (`/api/v1/customers`)

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /` · `GET /{id}` | `customers.read` | tìm theo tên (không dấu), SĐT (chuẩn hoá rồi băm), email; lọc nhóm, hạng, nhãn; theo phạm vi; liên hệ đã che (`phone_masked`, `email_masked`, D-226) |
| `GET /{id}/contact` | `customers.contact.reveal` | SĐT, email đầy đủ; ghi audit mỗi lần (D-226) |
| `POST /` | `customers.manage` | đăng ký tại quầy theo SĐT; trùng `phone_key` → 409 kèm `id` hồ sơ có sẵn |
| `PUT /{id}` | `customers.manage` | nhãn, tên, ngày sinh; `If-Match` |
| `POST /{id}/notes` · `GET /{id}/notes` | `customers.manage` / `customers.read` | ghi chú nội bộ |
| `GET /{id}/orders` | `customers.read` | từ `customers_order_facts` |
| `GET/POST /groups` · `PUT/DELETE /groups/{id}` · `POST/DELETE /groups/{id}/members/{profile_id}` | `customers.segments.manage` | quy tắc kiểm hợp lệ, xem trước số hồ sơ khớp (`GET /groups/{id}/preview`) |
| `GET/POST /loyalty/rules` · `GET/POST /loyalty/tiers` | `customers.loyalty.manage` | dữ liệu có ngày hiệu lực; không sửa dòng đã hiệu lực |
| `GET /{id}/points` · `POST /{id}/points/adjustments` | `customers.read` / `customers.loyalty.adjust` | điều chỉnh tay có `reference` + lý do (D-184) |
| `POST /gift-cards` · `GET /gift-cards/{id}` · `POST /gift-cards/lookup` · `POST /gift-cards/{id}/void` | `customers.stored_value.manage` | chỉ khi đã bật; tra mã bằng `POST` (mã không nằm trong URL / log) |
| `GET/PUT /gift-card-products/{variant_id}` | `customers.stored_value.manage` | |
| `POST /{id}/store-credit/adjustments` | `customers.stored_value.manage` | bồi thường có lý do, `reference` |
| `GET/POST /reminder-rules` · `GET/POST /referral-rules` | `customers.engagement.manage` | |
| `GET /me` · `PUT /me` | `customers.self` | hồ sơ của chính mình (tạo khi chưa có) |
| `GET /me/points` · `GET /me/gift-cards` · `POST /me/gift-cards/attach` · `GET /me/store-credit` | `customers.self` | gắn thẻ vào tài khoản (vẫn chỉ dùng trong khách) |
| `GET/POST /me/wishlist` · `DELETE /me/wishlist/{product_id}` | `customers.self` | |
| `POST /me/wishlist/merge` `{items}` | `customers.self` | gộp danh sách lưu ở trình duyệt khi đăng nhập, bỏ trùng (B-c13) |
| `GET /me/wishlist/alerts` · `PUT /me/wishlist/alerts` `{back_in_stock, price_drop, consent?: {text, text_version}}` | `customers.self` | báo yêu thích (D-233): bật cần `consent` → ghi sổ đồng ý; tắt cả hai → rút đồng ý, xoá hàng đợi |
| `GET /me/recently-viewed` · `POST /me/recently-viewed` | `customers.self` | |
| `GET /me/referral` · `POST /me/referral/apply` | `customers.self` | mã của mình; nhập mã người giới thiệu |

`customers.self` nằm trong role người mua mặc định (cùng chỗ với `orders.buy`).

### Tool MCP

`search_customers`, `get_customer` (`read_only`), `tag_customer`, `add_customer_note`, `list_groups`
(`read_only`), `upsert_group`, `preview_group` (`read_only`), `adjust_points` (`destructive`),
`customer_points` (`read_only`), `reveal_contact` (`customers.reveal_contact`, permission
`customers.contact.reveal`, cùng service với `GET /{id}/contact`, ghi audit — D-226). Tool trả danh sách / chi
tiết hồ sơ mang liên hệ đã che. Không có tool phát / huỷ thẻ quà tặng (việc có tiền, làm qua REST có
người duyệt) — ghi lý do trong quyết định. Không có tool bật báo yêu thích thay người mua: đồng ý phải do
chính người mua (D-233).

### Event

`customers.profile_created`, `customers.profile_updated`, `customers.profile_linked` (gắn tài khoản),
`customers.group_membership_changed`, `customers.points_earned`, `customers.points_redeemed`,
`customers.points_expired`, `customers.points_adjusted`, `customers.tier_changed`,
`customers.gift_card_issued`, `customers.gift_card_redeemed`, `customers.gift_card_voided`,
`customers.store_credit_changed`, `customers.referral_qualified`, `customers.referral_rewarded`,
`customers.cart_reminder_sent`, `customers.wishlist_alert_sent` (hồ sơ, số sản phẩm, loại — D-233). Payload
không có mã thẻ, SĐT rõ.

Nghe: `orders.order_placed`, `orders.order_confirmed`, `orders.order_cancelled`,
`orders.order_completed`, `orders.refund_completed`, `orders.cart_abandoned`, `privacy::USER_DELETED`,
sự kiện rút đồng ý của `notifications` (huỷ mọi nhắc giỏ đang chờ của người đó). Báo yêu thích (D-233):
`compliance.consent_withdrawn` (mục đích `customers.wishlist_alerts`), `catalog.price_changed`,
`catalog.price_effective`, event giá sau giảm tự động của `catalog`, `inventory.level_changed`
(`Subscription::optional`).

### Job và lịch

`customers.expire_points` (mỗi ngày), `customers.release_expired_holds` (mỗi 5 phút — điểm / thẻ giữ
quá hạn), `customers.evaluate_groups` (hằng đêm + enqueue theo hồ sơ, khoá có khe thời gian), `customers.evaluate_tiers` (mỗi
ngày), `customers.send_cart_reminder` (enqueue với `run_at`, `unique_key` = `cart:<id>:<step>`),
`customers.expire_gift_cards` (mỗi ngày, khi đã bật), `customers.purge_recent_views` (mỗi ngày),
`customers.post_ledger` (enqueue cùng transaction, `unique_key` = ref bút toán), cấp voucher sinh nhật (mỗi
ngày, theo múi giờ của khách, `source_ref = birthday:<năm>` — D-222), `customers.send_wishlist_alerts` (hằng
giờ, `unique_key` có khe giờ, chỉ gom trong khung giờ quảng cáo — D-233).

### Cấu hình

`AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED` (`false`), `AUTOXIDE_CUSTOMERS_WISHLIST_MAX` (200),
`AUTOXIDE_CUSTOMERS_RECENT_VIEWS_MAX` (50), `AUTOXIDE_CUSTOMERS_RECENT_VIEWS_DAYS` (90),
`AUTOXIDE_CUSTOMERS_REFERRAL_APPLY_DAYS` (14), `AUTOXIDE_CUSTOMERS_HOLD_MINUTES` (bằng hạn thanh toán
của `orders` + grace; khuyến nghị đọc từ cùng giá trị), `AUTOXIDE_CUSTOMERS_WISHLIST_PRICE_DROP_MIN_BP` (500 =
giảm ≥ 5%), `AUTOXIDE_CUSTOMERS_WISHLIST_ALERTS_PER_DAY` (1 tin / người / ngày) (D-233). Tính năng:
`customers.loyalty`, `customers.gift_cards`, `customers.store_credit`, `customers.referrals`,
`customers.cart_reminders`, `customers.wishlist_alerts` (D-233). Hạn mức: `customers.profiles`.

### Permission

`customers.self`, `customers.read`, `customers.manage`, `customers.segments.manage`,
`customers.loyalty.manage`, `customers.loyalty.adjust`, `customers.stored_value.manage`,
`customers.engagement.manage`, `customers.contact.reveal` (xem SĐT / email đầy đủ, D-226).

### Mã lỗi

| Code | HTTP | Tiếng Việt |
|---|---|---|
| `customers.profile_not_found` | 404 | Không tìm thấy hồ sơ khách hàng. |
| `customers.phone_taken` | 409 | Số điện thoại này đã có hồ sơ khách hàng. |
| `customers.phone_invalid` | 422 | Số điện thoại không hợp lệ. |
| `customers.group_name_taken` | 409 | Đã có nhóm tên {name}. |
| `customers.rule_invalid` | 422 | Quy tắc nhóm không hợp lệ: {detail}. |
| `customers.rule_effective_in_past` | 422 | Không thêm quy tắc có hiệu lực trong quá khứ. |
| `customers.insufficient_points` | 409 | Không đủ điểm (còn {available}, cần {requested}). |
| `customers.redeem_below_minimum` | 422 | Cần tối thiểu {min} điểm để đổi. |
| `customers.redeem_above_limit` | 422 | Điểm đổi tối đa {max_percent}% giá trị đơn. |
| `customers.stored_value_disabled` | 403 | Thẻ quà tặng và số dư cửa hàng chưa được bật. |
| `customers.gift_card_not_found` | 404 | Mã thẻ quà tặng không đúng. |
| `customers.gift_card_inactive` | 409 | Thẻ quà tặng đã hết hạn hoặc đã huỷ. |
| `customers.gift_card_out_of_scope` | 409 | Thẻ quà tặng này không dùng được ở cửa hàng này. |
| `customers.insufficient_balance` | 409 | Số dư không đủ (còn {available}). |
| `customers.face_value_too_high` | 422 | Giá trị thẻ tối đa {max}. |
| `customers.lookup_throttled` | 429 | Tra mã quá nhiều lần; thử lại sau {seconds} giây. |
| `customers.wishlist_full` | 422 | Danh sách yêu thích có tối đa {max} sản phẩm. |
| `customers.wishlist_alert_consent_required` (D-233) | 422 | Cần đồng ý nhận thông báo về sản phẩm yêu thích trước khi bật. |
| `customers.wishlist_alert_unavailable` (D-233) | 422 | Cửa hàng chưa quản lý tồn kho nên chưa báo có hàng lại được. |
| `customers.referral_code_invalid` | 422 | Mã giới thiệu không đúng. |
| `customers.referral_not_eligible` | 409 | Tài khoản không còn đủ điều kiện nhập mã giới thiệu. |
| `customers.self_referral` | 422 | Không tự giới thiệu chính mình được. |

## Thứ tự làm

Số **mục** theo danh sách bước 47 trong ROADMAP.

1. Khung crate, permission, cấu hình, `api.rs`, đăng ký. → chuẩn bị
2. Hồ sơ, nhận diện theo tài khoản / SĐT, `order_facts` từ event (net theo D-226), ghi chú, nhãn, tìm kiếm,
   che liên hệ và `GET /{id}/contact`. → **mục 1**
3. Nhóm + quy tắc + `BuyerGroups` (`groups_of`, `tier_of`) cắm vào `orders`. → **mục 2**
4. Điểm: quy tắc, lô, sổ, số dư, tender `points`, hết hạn, hạng, bút toán. → **mục 3**
5. Thẻ quà tặng và số dư sau hai lớp tắt, tender, bút toán, `doctor`. → **mục 4**
6. Yêu thích (kể cả gộp từ trình duyệt), vừa xem, nhắc giỏ bỏ dở theo đồng ý, giới thiệu bạn. → **mục 5**
7. Voucher qua `grant_voucher` (sinh nhật, đổi điểm, sau khi mua, giới thiệu bạn). → **mục 6**
8. Báo yêu thích có hàng lại / giảm giá (D-233): cài đặt và đồng ý, giá mốc, nghe event giá và tồn (thêm event /
   hàm đọc giá hiển thị vào `catalog` nếu bước 41 chưa có), hàng đợi, job gom lô, loại thông báo. → **mục 7**
9. `PersonalData` (gồm cài đặt, hàng đợi, lô báo yêu thích), `USER_DELETED`, tool MCP, `docs/openapi.json`,
   `.env.example`, CHANGELOG, ROADMAP, quyết định.

## Test bắt buộc

Cả SQLite và Postgres, `TestApp` với `orders` và module nền thật; `Notifier` giả ghi lại tin và đồng ý.

- `profile_built_from_order_events_once` — event giao hai lần: tổng chi, số đơn không cộng đôi.
- `phone_profile_links_only_to_verified_phone_account`.
- `rule_group_membership_updates_and_feeds_orders_pricing` — vào nhóm → `orders` báo giá theo bảng giá nhóm.
- `points_earned_on_completion_not_on_payment`; `points_expire_fifo_by_lot`.
- `concurrent_redemptions_never_overdraw` — 20 checkout song song cùng dùng điểm: Σ đổi ≤ số dư, số
  dư không âm, lô không âm.
- `points_hold_released_when_order_cancelled_or_hold_expires`.
- `points_ledger_liability_equals_outstanding_points_value` — sau chuỗi tích / đổi / hết hạn / hoàn:
  số dư `loyalty_liability` = Σ (điểm còn × giá trị lúc tích).
- `stored_value_disabled_by_default` — mọi endpoint 403, không có tender trong checkout.
- `gift_card_cannot_be_used_in_other_tenant`; `gift_card_has_no_cash_out_path` (duyệt OpenAPI: không
  endpoint nào giảm số dư ngoài hold / void / expire).
- `gift_card_lookup_is_throttled`; `concurrent_gift_card_redemptions_never_overdraw`.
- `cart_reminder_only_with_marketing_consent`; `consent_withdrawn_cancels_pending_reminders`;
  `order_placed_cancels_reminder`.
- `referral_rewarded_once_after_first_order_completed`; `self_referral_same_phone_rejected`.
- Voucher (D-222): `voucher_sinh_nhat_moi_nam_mot_lan`, `doi_diem_lay_voucher_khong_nhan_doi`,
  `cap_voucher_loi_hoan_diem`.
- Báo yêu thích (D-233; `Notifier` giả, `ConsentLedger` thật, giờ giả lập):
  - `bao_yeu_thich_mac_dinh_tat_va_chua_dong_y_khong_gui`: chưa bật không gửi; bật mà thiếu `consent` → 422;
    cờ bật nhưng sổ không còn đồng ý → không gửi.
  - `bao_giam_gia_duoi_nguong_khong_gui`: giảm 4,99% không gửi, 5% gửi; tăng rồi giảm về mốc không gửi; giá gạch
    `compare_at` đổi không gửi; sau khi báo, mốc = giá đã báo.
  - `bao_yeu_thich_event_lap_khong_gui_hai_lan`: cùng event giao hai lần, và lần nữa trong 7 ngày → một tin;
    sau 7 ngày giảm tiếp đủ ngưỡng → tin mới.
  - `bao_yeu_thich_hai_job_dong_thoi_mot_tin`: hai worker chạy lô cùng lúc cho một hàng đợi → một tin, cập nhật
    có điều kiện thắng một lần.
  - `bao_yeu_thich_gom_lo_mot_tin`: ba sản phẩm trong một giờ → một tin ba dòng; vượt `WISHLIST_ALERTS_PER_DAY`
    thì chờ hôm sau.
  - `rut_dong_y_bao_yeu_thich_dung_ngay`: rút qua `compliance` sau khi vào hàng đợi → không gửi, hàng đợi rỗng,
    cờ tắt.
  - `bao_yeu_thich_ngoai_khung_gio_cho_lo_sau`: 23:00 giờ của khách không gom; 07:00 gom và gửi.
  - `bao_co_hang_lai_chi_khi_tu_het_len_con`: tồn 0 → 3 báo; 3 → 5 không; thiếu `inventory` vẫn khởi động được.
- `export_contains_profile_points_wishlist`; `user_deleted_removes_personal_fields_keeps_ledger`.
- Rò rỉ: `tenant_isolation`; `store_staff_sees_only_own_branch_profiles` (phạm vi `Unit` / `Subtree`).

## Tiêu chí xong

- [ ] Mọi mục của bước 47 có commit và test; gate đầy đủ xanh trên cả hai database.
- [ ] Thẻ quà tặng / số dư tắt mặc định; bật cần cả biến môi trường và tính năng theo khách; `doctor`
      cảnh báo khi bật.
- [ ] Nghĩa vụ điểm / thẻ / số dư khớp sổ cái (test).
- [ ] Quyết định trong `docs/DECISIONS.md`: nhận diện theo SĐT (HMAC theo khách), cách tính nghĩa vụ
      điểm, luật cứng của thẻ quà tặng kèm căn cứ NĐ 52/2024 và "cần đối chiếu", nhắc giỏ theo NĐ 91/2020.
- [ ] `docs/openapi.json`, `.env.example`, CHANGELOG, ROADMAP; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Chiếm điểm qua SĐT**: chỉ gắn hồ sơ theo SĐT đã xác thực; nhân viên đăng ký tại quầy không làm hồ
  sơ "thuộc" tài khoản nào cho tới khi chủ SĐT xác thực.
- **Tiêu lô đồng thời**: khoá lô theo `id` tăng dần trong một transaction; giảm số dư tổng bằng
  `rows::adjust` trước, thất bại → `insufficient_points` trước khi đụng lô.
- **Giá trị điểm đổi theo thời gian**: nghĩa vụ ghi theo giá trị lúc tích (lưu trên lô); đổi điểm ghi
  theo giá trị của lô bị tiêu, không theo quy tắc hiện tại — không thì sổ cái lệch.
- **Hoàn tiền sau khi điểm đã tiêu**: không để số dư âm; phần không thu hồi ghi chi phí, có event để
  nhân viên biết.
- **Nhận lại event** (`order_completed` hai lần → tích hai lần): khoá theo `source_ref` unique
  (`orders:<order_id>:earn`).
- **Tin quảng cáo**: kiểm đồng ý lúc lên lịch **và** lúc gửi (có thể đã rút giữa chừng); không gửi
  cho vãng lai không có đồng ý ghi trong sổ.
- **Dữ liệu cá nhân**: SĐT, ngày sinh, ghi chú nhân viên là dữ liệu cá nhân — có trong export, xoá khi
  `USER_DELETED`; phần sổ cái (nghĩa vụ) giữ theo luật kế toán, không mang tên / SĐT.
- **Giờ hết hạn điểm** theo múi giờ của khách (cuối ngày giờ Việt Nam), lưu UTC. Voucher sinh nhật cũng xét
  theo múi giờ của khách.
- **Đổi điểm lấy voucher**: trừ điểm và cấp voucher dùng chung `source_ref`; `grant_voucher` lỗi thì hoàn điểm
  bằng bút toán đảo, không để điểm mất mà không có voucher (D-222).
- **Lộ liên hệ**: mọi đường cho nhân viên (REST, MCP, xuất danh sách) trả bản đã che; chỉ `GET /{id}/contact`
  (và xuất có permission `customers.contact.reveal`) trả đủ, có audit (D-226).
- **Báo giảm giá ảo** (D-233): so với giá đã thật sự hiển thị, không với giá gạch; không nâng mốc khi giá tăng
  (nâng giá rồi hạ về là giảm ảo); ngưỡng tính bằng số nguyên, không `f64`.
- **Event của báo yêu thích đến lại / đến trễ** (D-233): handler đọc lại giá / tồn hiện tại; job kiểm lại
  lần nữa trước khi gửi; `<loại>_alerted_at` cập nhật có điều kiện — không dựa vào "đã có tin trong hộp" của
  `notifications`.
- **Gọi `notify` trong transaction ghi** (D-233): commit lô trước, gọi sau, `dedupe_key` theo lô để retry không
  gửi hai lần.
- **Lẫn với báo có hàng lại của `inventory`** (D-201): đó là đăng ký theo biến thể, cả khách vãng lai, mục đích
  đồng ý riêng; mục này là danh sách yêu thích của người đã đăng nhập. Hai sổ, hai mục đích — không gộp.

## Căn cứ pháp lý

- NĐ 52/2024/NĐ-CP (thanh toán không dùng tiền mặt) về ví điện tử, thẻ trả trước: thẻ quà tặng / số dư
  có thể bị coi là phương tiện thanh toán nếu dùng ngoài phạm vi người phát hành hay đổi ra tiền —
  **cần đối chiếu văn bản gốc và người có chuyên môn pháp lý trước khi bật** (D-194). Thiết kế giữ
  khép kín: chỉ người phát hành, không rút, không chuyển.
- NĐ 91/2020/NĐ-CP (chống tin nhắn rác, thư điện tử rác): tin quảng cáo chỉ gửi khi đã đồng ý trước,
  dừng ngay khi từ chối, khung giờ (7h–22h, là dữ liệu của `notifications`) — cần đối chiếu bản thay
  thế đang dự thảo.
- Luật BVDLCN 91/2025 + NĐ 356/2025: hồ sơ, phân khúc, lịch sử mua là xử lý dữ liệu cá nhân — ghi căn
  cứ / mục đích trong sổ của `compliance`; quyền xem, xoá; phân khúc để tiếp thị cần đồng ý — cần đối chiếu.
  Tối thiểu hoá, kiểm soát truy cập → che SĐT / email, xem đủ có permission và audit (D-226) — **cần đối chiếu
  văn bản gốc**.
- Luật Kế toán 88/2015: nghĩa vụ điểm / thẻ / số dư là số liệu kế toán, giữ ít nhất 10 năm.
- Luật 19/2023/QH15: điều kiện của chương trình điểm, hạng, giới thiệu là điều kiện giao dịch — công
  khai cho người mua (API trả quy tắc hiện hành).
- Báo yêu thích (D-233): tin quảng cáo theo NĐ 91/2020/NĐ-CP (đồng ý trước, dừng ngay khi từ chối, khung giờ);
  đồng ý theo mục đích riêng, rút bất kỳ lúc nào (Luật BVDLCN 91/2025/QH15, NĐ 356/2025); không báo giảm giá ảo —
  thông tin về giá phải trung thực (Luật 19/2023/QH15) — **cần đối chiếu văn bản gốc / người có chuyên môn
  pháp lý**.

## Agent làm bước này tự quyết

- Tích điểm khi `completed` (khuyến nghị) hay khi `delivered` — ghi quyết định.
- Hạn mặc định của điểm trả lại khi hoàn tiền (khuyến nghị như trên).
- Báo hàng về / giảm giá cho danh sách yêu thích: đã chốt ở D-233, làm ở mục 7 — xem "Báo khi sản phẩm yêu
  thích có hàng lại hay giảm giá". Tự chọn: số sản phẩm tối đa một tin (khuyến nghị 10), kênh mặc định.
- Thưởng giới thiệu mặc định: khuyến nghị điểm (không cần thẻ quà tặng đã bật).
- Nếu bước 45 chưa thêm `contact_key` vào kernel: thêm ở bước này (một hàm, có test hai khách cho hai
  khoá khác nhau) và ghi quyết định.
