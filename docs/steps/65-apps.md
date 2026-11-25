# Bước 65 — Chợ ứng dụng qua OAuth 2.0 (`apps`)

> Lộ trình: `docs/ROADMAP.md` bước 65 · Quyết định: D-221 (thay mục "để sau" của D-205), D-232 · Đặc tả liên
> quan: 54 (phân quyền), 40 (MCP), 33 (khách), 35 (thuê bao, kỳ thanh toán), 36 (sổ cái, `PayoutGateway`, thuế,
> hoá đơn), 37 (KYC, văn bản có phiên bản)

## Mục tiêu

Nhà phát triển bên ngoài viết ứng dụng (kế toán, giao hàng, email marketing, AI agent) cắm vào Autoxide như
Shopify App Store: khách bấm "cài", xem ứng dụng xin những quyền gì, đồng ý; ứng dụng nhận token **chỉ trong
các quyền đó, chỉ trong khách đó**, gọi REST / MCP như mọi người gọi khác; khách gỡ là token mất hiệu lực ngay.
Ứng dụng miễn phí hay thuê bao tháng có dùng thử; nền tảng là bên bán, thu phí ứng dụng trên hoá đơn thuê bao
của khách và chia doanh thu cho nhà phát triển đã xác thực (D-232).

## Bắt đầu khi

Bước 54 (phân quyền chuẩn: hợp đồng quyền máy đọc, thu hồi tức thì, key chi tiết) và 33 (khách). Phần ứng dụng
trả phí cần thêm 35 (thuê bao, kỳ thanh toán), 36 (`Ledger`, `PayoutGateway`, `TaxEngine`, `invoices`) và 37
(`KycGate`, `AcceptanceGate`) (D-232).

## Thiết kế

- **Máy chủ ủy quyền OAuth 2.0 trong `auth`** theo OAuth 2.1 (bản nháp của IETF, gom RFC 6749 + thực hành tốt):
  - chỉ luồng `authorization_code` + **PKCE bắt buộc** (RFC 7636) và `refresh_token` xoay vòng; luồng
    `client_credentials` cho ứng dụng máy–máy của chính khách;
  - không có luồng implicit, không password grant;
  - `redirect_uri` khớp chính xác đã đăng ký; `state` bắt buộc;
  - phạm vi (`scope`) = **tên permission** (`orders.read`, `catalog.write`…), kèm độ sâu / nút tối đa (bước 34);
  - token truy cập ngắn hạn (mặc định 1 giờ), không mang quyền trong token (thu hồi tức thì, D-205 P11);
  - endpoint chuẩn: `/oauth/authorize`, `/oauth/token`, thu hồi (RFC 7009), kiểm tra token (RFC 7662), tài liệu
    máy chủ (RFC 8414 `/.well-known/oauth-authorization-server`); chỉ báo tài nguyên (RFC 8707) cho MCP.
- **Ứng dụng** (`apps_apps`: chủ ứng dụng (nhà phát triển), tên, mô tả, logo, trang chính sách quyền riêng tư,
  `redirect_uris`, quyền xin, webhook nhận, trạng thái duyệt `draft/in_review/published/suspended`). Ứng dụng
  công khai trên chợ cần **nền tảng duyệt** (kiểm tra quyền xin có tương xứng, chính sách dữ liệu); ứng dụng
  riêng của một khách không cần duyệt.
- **Cài đặt** (`apps_installations`: khách, ứng dụng, quyền đã đồng ý, nút giới hạn, người đồng ý, thời điểm,
  `revoked_at`). Màn hình đồng ý do giao diện dựng từ `GET /api/v1/apps/consent?client_id&scope`, liệt kê quyền
  bằng mô tả của permission (đã có trong catalog), đánh dấu quyền rủi ro (`risk` của D-205). Ứng dụng muốn thêm
  quyền → khách đồng ý lại.
- **Webhook theo ứng dụng:** đăng ký qua module `webhooks` sẵn có, chỉ event ứng dụng có quyền đọc.
- **Chặn lạm dụng:** rate limit theo ứng dụng × khách; audit ghi `actor_kind = app` + `app_id`; khách xem nhật ký
  gọi của ứng dụng (bàn kỹ thuật bước 52); nền tảng tạm khoá ứng dụng vi phạm (token của mọi khách mất hiệu lực).
- **Thanh toán cho ứng dụng trả phí:** xem mục "Ứng dụng trả phí và chia doanh thu" bên dưới (D-232). Ứng dụng
  vẫn được tự thu ngoài (nhà phát triển tự bán gói của mình); khi đó giá trên chợ là `free`.

### Ứng dụng trả phí và chia doanh thu (D-232)

**Giá.** Mỗi ứng dụng có giá `free` hay `monthly` (thuê bao tháng theo khách) — không có trả một lần, không tính
theo lượt dùng ở bước này — kèm `trial_days` (dùng thử N ngày, 0 = không). Giá khai **chưa gồm GTGT**; thuế do
`invoices` tính trên dòng hoá đơn theo `tax_category`. `apps_prices` chỉ thêm: giá hiện hành = dòng có
`effective_from` lớn nhất ≤ `now`. Ứng dụng có giá `monthly` chỉ công bố được khi nhà phát triển đã đủ điều kiện
(mục "Nhà phát triển") và có tỉ lệ chia doanh thu hiệu lực.

**Nền tảng là bên bán (merchant of record).** Khách mua ứng dụng **của nền tảng** (khách vận hành của 35): phí
ứng dụng là một dòng thêm trên hoá đơn thuê bao của khách (bước 35), hoá đơn điện tử do `invoices` xuất cho khách
(36). Nền tảng thu qua cổng của khách vận hành (đối tác có giấy phép, D-187); tiền thu là phí dịch vụ của chính
nền tảng — nền tảng không giữ tiền nào khác của khách. Phần của nhà phát triển là khoản nền tảng phải trả, chi
hằng tháng qua `PayoutGateway`.

**Kỳ và cách thu.** Kỳ của ứng dụng là tháng neo theo ngày bắt đầu kỳ thuê bao gói của khách (gói theo tháng: trùng
kỳ gói). Thu trước theo kỳ; phần lẻ đầu tiên (cài hay hết dùng thử giữa kỳ) tính theo tỉ lệ số giây còn lại của
kỳ, làm tròn xuống (công thức `proration` của 35), và thu ở kỳ kế. `plans` lấy phí qua điểm cắm của kernel:

```rust
// autoxide_kernel::entitlement — apps (đăng ký TRƯỚC plans) gọi `context.contribute::<dyn AddonBilling>(…)`;
// plans đọc `StartContext::contributions::<dyn AddonBilling>()`, không phụ thuộc crate apps.
pub struct AddonCharge { pub reference: String /* "apps:charge:<id>" */, pub description: String,
    pub amount: Money, pub tax_category: Option<String>, pub period_start: DateTime<Utc>, pub period_end: DateTime<Utc> }
#[async_trait] pub trait AddonBilling: Send + Sync {
    /// Phí của khách cho kỳ [start, end) cộng phần lẻ chưa thu; gắn với `billing_period_id` trong một transaction
    /// của apps. Gọi lại cùng `billing_period_id` trả đúng tập cũ (không tính hai lần).
    async fn charges_for_period(&self, tenant: TenantId, billing_period_id: Uuid, start: DateTime<Utc>,
        end: DateTime<Utc>) -> Result<Vec<AddonCharge>, Error>;
}
```

`plans.renewals` gọi hàm này khi lập kỳ thanh toán kế và thêm mỗi khoản thành một dòng (tên ứng dụng, kỳ, đơn
giá, nhóm thuế). Gói theo năm: `plans` lập thêm kỳ `addons` mỗi tháng tại ngày neo chỉ để thu phí ứng dụng.
`apps` nghe `plans.billing_period_paid` → khoản phí `paid`; `plans.billing_period_voided` → `void`. Khách không
trả kỳ có phí ứng dụng thì theo vòng đời thuê bao gói (35): khách chỉ đọc thì ứng dụng cũng chỉ đọc.

**Vòng đời thuê bao ứng dụng** (`apps_subscriptions`, một dòng cho mỗi (khách, ứng dụng), sống qua nhiều lần cài;
mọi chuyển trạng thái `rows::lock_one` dòng thuê bao rồi mới đổi — job và lệnh của khách không đè nhau):

| Từ | Sự việc | Sang |
|---|---|---|
| — | cài ứng dụng `monthly` có `trial_days` > 0, chưa dùng thử ứng dụng này | `trialing` (`trial_ends_at`, `trial_used = true`) |
| — | cài ứng dụng `monthly`, không còn dùng thử | `active` (phần lẻ tới hết kỳ) |
| `trialing` | hết dùng thử (job `apps.lifecycle`) | `active` (phần lẻ tới hết kỳ, thu ở kỳ kế) |
| `trialing` | gỡ | `canceled` ngay, không thu gì |
| `active` | gỡ | `canceling` (`cancel_at_period_end`); token mất hiệu lực ngay như ứng dụng miễn phí |
| `canceling` | hết kỳ | `canceled` — không thu kỳ sau, **không hoàn phần đã dùng** trừ khi chính sách hoàn tiền nói khác |
| `canceling` | cài lại trước khi hết kỳ | `active` (không thu thêm, không dùng thử lại) |

Khách đồng ý giá khi cài: `GET /consent` trả giá, số ngày dùng thử; thân `POST /installations` mang
`accepted_price_id` — khác giá hiện hành → `apps.price_changed`. Máy chủ không có `plans` (một khách tự vận hành,
`EntitlementsHandle` chưa gắn) → chợ chỉ bán ứng dụng `free`, cài ứng dụng có phí → `apps.billing_unavailable`.

**Nhà phát triển đổi giá.** Thêm dòng `apps_prices` mới với `effective_from` ≥ `now` +
`AUTOXIDE_APPS_PRICE_NOTICE_DAYS` (ngắn hơn → `apps.price_notice_too_short`) → event
`apps.price_change_scheduled`, thông báo (tin giao dịch) tới quản trị mọi khách đang dùng. Khách đang dùng giữ giá
cũ cho kỳ đang chạy và mọi kỳ bắt đầu trước `effective_from`; kỳ đầu tiên bắt đầu từ `effective_from` dùng giá mới
(`apps_subscriptions.price_id` đổi ở đầu kỳ, trong `apps.lifecycle`). Chuyển `free` → `monthly`: khách đang cài phải
đồng ý giá (`POST /installations/{id}/accept-price`) trước `effective_from`; không đồng ý thì tới hạn cài đặt bị tạm
ngưng (token mất hiệu lực), **không tự tính tiền**.

**Chia doanh thu.** `revenue_share_bp` (phần của nhà phát triển, điểm cơ bản) là dữ liệu có ngày hiệu lực theo
thoả thuận (`apps_revenue_shares`, chỉ thêm): tìm theo (nhà phát triển, ứng dụng) → (nhà phát triển) → mặc định
nền tảng, hiệu lực tại `period_start` của khoản phí. Khi khoản phí `paid`: `developer_share_minor =
floor(amount × bp / 10000)`, `platform_share_minor = amount − developer_share_minor` (tính bằng `i128`, không làm
tròn hai lần) — hai phần cộng lại đúng bằng khoản phí, từng đồng. Bút toán trên sổ của khách vận hành (bộ tên của
36, `reference` = `apps:charge:<id>:share`): Nợ `apps.revenue_share` (chi phí chia doanh thu, mục đích riêng có
tiền tố module) / Có `payable.supplier` (đối tượng `partner:<developer_id>`). Doanh thu và GTGT của dòng phí do
`invoices` ghi như mọi dòng của hoá đơn thuê bao — `apps` không ghi lại.

**Thời gian hoàn tiền và chi trả.** Khoản phí `paid` thành `releasable` sau `AUTOXIDE_APPS_REFUND_WINDOW_DAYS` (job
hằng ngày `apps.release_earnings`). Nền tảng hoàn phí trong thời gian đó (`POST /billing/charges/{id}/refund`, tiền
trả khách qua `ledger` và hoá đơn điều chỉnh của `invoices`) → `refunded`, đảo bút toán chia; hoàn sau khi đã chi →
số âm trừ vào lần chi sau. Job `apps.build_payouts` (chạy ngày `AUTOXIDE_APPS_PAYOUT_DAY` hằng tháng; `unique_key`
`apps.build_payouts:<yyyy-mm>`), mỗi (nhà phát triển, tiền tệ) một transaction: nhà phát triển **đủ điều kiện chi**
(dưới đây) → gom khoản `releasable` (`UPDATE … SET status = 'in_payout', payout_id = :p WHERE status =
'releasable'` — không đọc rồi ghi) trừ số âm còn nợ; ≤ 0 thì bỏ qua → `apps_payouts` (`reference` =
`apps:payout:<developer_id>:<yyyy-mm>:<currency>`, unique). Khấu trừ thay theo `TaxEngine::withholding` khi hồ sơ
thuế là hộ / cá nhân kinh doanh (như 46): Nợ `payable.supplier[D]` / Có `tax.withheld_vat`, `tax.withheld_pit` —
cần đối chiếu. Sau commit gọi `Ledger::create_payout` (36) → `PayoutGateway` ngoài transaction;
`ledger.payout_sent` → `paid_out` (bút toán chi do `ledger` ghi: Nợ `payable.supplier[D]` / Có
`clearing.gateway`); `ledger.payout_failed` → khoản phí trở lại `releasable`. Nhà phát triển chưa đủ điều kiện:
khoản phí ở lại `releasable`, cộng dồn tới khi đủ — **không bao giờ chi** cho nhà phát triển chưa xác thực.

**Nhà phát triển.** `apps_developers`: hồ sơ (loại `individual` / `household_business` / `organization`, tên pháp
lý, quốc gia, mã số thuế, địa chỉ), tài khoản nhận tiền (mã hoá bằng `seal` của kernel, chỉ hiện 4 số cuối). Xác
thực ở `compliance` của khách vận hành: KYC (`KycGate::status` của chủ hồ sơ là `verified` còn hạn — kiểm trực
tiếp, không theo ngày bắt đầu của `kyc.required_activities`), chấp nhận phiên bản hiện hành của **thoả thuận nhà
phát triển** (văn bản có phiên bản của 37, loại `developer_agreement`, `AcceptanceGate`). Đủ điều kiện chi = KYC
`verified` + đã chấp nhận thoả thuận + có thông tin thuế + có tài khoản nhận tiền + quốc gia được mở. **Nhà phát
triển nước ngoài**: thuế nhà thầu / khấu trừ khi nền tảng chi ra nước ngoài **cần đối chiếu**; tới khi đối chiếu,
`AUTOXIDE_APPS_FOREIGN_DEVELOPERS` = `false` → nhà phát triển có quốc gia khác `VN` chỉ đăng ứng dụng `free`
(`apps.developer_country_not_supported`).

| Bảng (mặt điều khiển — cùng chỗ với `plans_*`, vì kỳ thanh toán và chi trả gom xuyên khách) | Cột chính | Khoá / index |
|---|---|---|
| `apps_prices` | `id`, `app_id`, `kind` (`free`/`monthly`), `amount_minor`, `currency`, `trial_days`, `tax_category` NULL, `effective_from`, `announced_at`, `created_by`, `created_at` | index (`app_id`,`effective_from`); chỉ thêm |
| `apps_revenue_shares` | `id`, `developer_id` NULL (NULL = mặc định nền tảng), `app_id` NULL, `revenue_share_bp`, `effective_from`, `effective_to` NULL, `agreement_ref` (văn bản / phụ lục), `created_by`, `created_at` | index (`developer_id`,`app_id`,`effective_from`); chỉ thêm — đổi = đóng `effective_to` + dòng mới; chồng nhau → `apps.revenue_share_overlap` |
| `apps_developers` | `id`, `owner_user_id`, `kind`, `legal_name`, `country` (ISO 3166-1), `tax_code`, `address` JSON, `payout_account` (AEAD), `payout_account_last4`, `key_version`, `created_at`, `updated_at` | unique (`owner_user_id`) |
| `apps_subscriptions` | `id`, `tenant_id`, `app_id`, `installation_id`, `status` (`trialing/active/canceling/canceled`), `price_id`, `trial_ends_at` NULL, `trial_used`, `current_period_start`, `current_period_end`, `cancel_at_period_end`, `canceled_at` NULL, `created_at`, `updated_at` | unique (`tenant_id`,`app_id`); index (`status`,`current_period_end`), (`status`,`trial_ends_at`) |
| `apps_charges` | `id`, `tenant_id`, `subscription_id`, `app_id`, `developer_id`, `kind` (`period`/`proration`), `period_start`, `period_end`, `price_id`, `amount_minor`, `currency`, `reference`, `billing_period_id` NULL, `status` (`pending/billed/paid/releasable/in_payout/paid_out/refunded/void`), `revenue_share_bp` NULL, `developer_share_minor` NULL, `platform_share_minor` NULL, `paid_at` NULL, `releasable_at` NULL, `payout_id` NULL, `refunded_at` NULL, `created_at`, `updated_at` | unique (`reference`); index (`tenant_id`,`billing_period_id`), (`developer_id`,`status`,`releasable_at`), (`payout_id`) |
| `apps_payouts` | `id`, `developer_id`, `period` (`yyyy-mm`), `currency`, `gross_minor`, `withheld_minor`, `amount_minor`, `ledger_payout_id` NULL, `status` (`pending/sent/succeeded/failed`), `reference`, `created_at`, `updated_at` | unique (`reference`); unique (`developer_id`,`period`,`currency`) |

### API, MCP, quyền, lỗi

- REST `/api/v1/apps`: nhà phát triển (`/developer/apps`, khoá bí mật xoay vòng, gửi duyệt), chợ công khai
  (`/public/apps`), khách (`/installations`, gỡ), nền tảng (`/review`).
- MCP: AI agent bên ngoài nối MCP bằng token OAuth của ứng dụng đã cài (chuẩn ủy quyền của MCP).
- Permission: `apps.install` (khách), `apps.develop` (nhà phát triển), `apps.review` (nhân sự nền tảng).
- Lỗi theo chuẩn OAuth (`invalid_request`, `invalid_scope`, `access_denied`…) cho endpoint OAuth; còn lại
  `apps.not_found`, `apps.scope_not_allowed`, `apps.suspended`.

**REST thêm cho ứng dụng trả phí** (`/api/v1/apps`, D-232):

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET/PUT /developer/profile` | `apps.develop` | hồ sơ, thuế, tài khoản nhận tiền; trả các bước còn thiếu (KYC, thoả thuận, thuế, tài khoản) |
| `GET/POST /developer/apps/{id}/prices` | `apps.develop` | thêm giá; `apps.price_notice_too_short`, `apps.developer_not_verified` |
| `GET /developer/earnings`, `GET /developer/payouts` | `apps.develop` | theo kỳ, trạng thái, lý do chưa chi |
| `GET /installations/{id}/subscription`, `POST /installations/{id}/accept-price` | `apps.install` | thuê bao, khoản phí, giá sắp đổi |
| `GET/POST /billing/revenue-shares` | `apps.billing.manage` | tỉ lệ có ngày hiệu lực |
| `GET /billing/charges`, `POST /billing/charges/{id}/refund` | `apps.billing.manage` | lý do bắt buộc; audit |
| `GET /billing/payouts`, `POST /billing/payouts/{id}/retry` | `apps.billing.manage` | chi lại lần lỗi |

`GET /public/apps/{id}` và `GET /consent` thêm giá hiện hành, `trial_days`, giá sắp đổi (D-232).

**Tool MCP thêm** (cùng service, cùng permission): `get_app_subscription` (`read_only`, `apps.install`),
`list_developer_earnings` (`read_only`, `apps.develop`). Không có tool cho hoàn phí, chi trả, tỉ lệ chia (D-232).

**Event thêm** (chỉ id, trạng thái, số tiền của chính khoản đó; không số tài khoản): `apps.subscription_started`,
`apps.subscription_status_changed` (`from`, `to`), `apps.trial_ending`, `apps.price_change_scheduled`,
`apps.charge_paid`, `apps.charge_refunded`, `apps.payout_created`, `apps.payout_succeeded`, `apps.payout_failed`.
Nghe: `plans.billing_period_paid`, `plans.billing_period_voided` (`Subscription::optional` — `plans` đăng ký sau),
`ledger.payout_sent`, `ledger.payout_failed`, `compliance.kyc_verified` / `…kyc_expired` / `…kyc_revoked` (D-232).

**Job thêm:** `apps.lifecycle` (5 phút: hết dùng thử, đầu kỳ áp giá mới, cuối kỳ dừng thuê bao đã gỡ, tạm ngưng
cài đặt không đồng ý giá), `apps.trial_reminders` (hằng ngày, trước khi hết dùng thử
`AUTOXIDE_APPS_TRIAL_REMINDER_DAYS` ngày; `unique_key` theo thuê bao), `apps.price_change_notices`,
`apps.release_earnings` (hằng ngày), `apps.build_payouts` (hằng ngày, chỉ làm vào ngày chi) (D-232).

**Cấu hình thêm** (cấp máy chủ — chợ ứng dụng là của nền tảng): `AUTOXIDE_APPS_PRICE_NOTICE_DAYS` (30),
`AUTOXIDE_APPS_REFUND_WINDOW_DAYS` (14), `AUTOXIDE_APPS_PAYOUT_DAY` (5; 1–28), `AUTOXIDE_APPS_TRIAL_REMINDER_DAYS`
(3), `AUTOXIDE_APPS_FOREIGN_DEVELOPERS` (`false`) (D-232).

**Permission thêm:** `apps.billing.manage` (nhân sự nền tảng: tỉ lệ chia, hoàn phí, chi trả) (D-232).

**Mã lỗi thêm** (D-232):

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `apps.billing_unavailable` | 409 | Máy chủ chưa bật thu phí thuê bao nên chưa cài được ứng dụng có phí. |
| `apps.price_changed` | 409 | Giá ứng dụng đã đổi; hãy xem lại giá trước khi cài. |
| `apps.price_notice_too_short` | 422 | Đổi giá phải báo trước ít nhất {days} ngày (hiệu lực sớm nhất {earliest}). |
| `apps.developer_not_verified` | 409 | Nhà phát triển chưa đủ điều kiện bán ứng dụng có phí, còn thiếu: {missing}. |
| `apps.developer_country_not_supported` | 422 | Chưa bán được ứng dụng có phí của nhà phát triển ngoài Việt Nam. |
| `apps.revenue_share_missing` | 409 | Chưa có tỉ lệ chia doanh thu hiệu lực cho ứng dụng này. |
| `apps.revenue_share_overlap` | 409 | Đã có tỉ lệ chia doanh thu hiệu lực từ {from} cho cùng đối tượng. |
| `apps.charge_not_refundable` | 409 | Khoản phí này không hoàn được: {reason}. |

## Test bắt buộc (cả hai database)

`thieu_pkce_bi_tu_choi`, `redirect_uri_khac_bi_tu_choi`, `token_chi_trong_quyen_da_dong_y`,
`token_khach_a_khong_dung_o_khach_b`, `go_ung_dung_token_mat_hieu_luc_ngay`, `refresh_token_dung_lai_bi_thu_hoi_ca_chuoi`,
`ung_dung_bi_khoa_moi_khach_mat_quyen`, `well_known_dung_rfc_8414`.

Ứng dụng trả phí (D-232; `FakePayoutGateway`, `FakeEInvoiceProvider`): `dung_thu_het_han_chuyen_tra_phi`,
`dung_thu_chi_mot_lan_moi_ung_dung`, `go_giua_ky_dung_cuoi_ky_khong_hoan`, `cai_lai_truoc_het_ky_khong_thu_them`,
`phi_ung_dung_la_dong_tren_hoa_don_thue_bao`, `goi_lai_charges_for_period_khong_tinh_hai_lan`,
`chia_doanh_thu_tong_khop_tung_dong` (Σ phần nhà phát triển + Σ phần nền tảng = Σ khoản phí; Σ chi + khấu trừ =
Σ phần nhà phát triển; số dư `payable.supplier[D]` = 0 sau chi), `khong_chi_cho_nha_phat_trien_chua_xac_thuc`,
`chi_sau_thoi_gian_hoan_tien`, `chi_tra_chay_dong_thoi_khong_chi_hai_lan`,
`doi_gia_khong_ap_ngay_cho_khach_dang_dung`, `doi_gia_bao_truoc_thieu_ngay_bi_tu_choi`,
`free_sang_tra_phi_can_khach_dong_y`, `nha_phat_trien_nuoc_ngoai_chi_dang_mien_phi`.

### Mini app (D-20261125-miniapp-platform)

Bước này là tầng "mini app" của nền: nhà phát triển bên ngoài dựng mini app trên core như trên Zalo Mini App hay
MoMo Mini App. Phần trên tả phía server (OAuth, cài, đồng ý, webhook, bán ứng dụng). Ba phần dưới đây làm nó
thành nền cho mini app có giao diện. Đây mới là yêu cầu, chưa phải thiết kế chi tiết: agent mở bước này viết thiết
kế vào đây trước khi làm, và ghi quyết định cho những lựa chọn có cách khác cũng hợp lý.

- **Bản mô tả mini app máy đọc được:** một tài liệu JSON có schema công bố (tên, phiên bản, nhà phát triển, quyền
  xin, `redirect_uris`, địa chỉ giao diện, webhook nhận, phiên bản API tối thiểu). Đăng ký ứng dụng nhận bản mô tả
  này thay cho điền tay từng trường. Cùng một bản mô tả phải cài được trên mọi bản cài Autoxide cùng phiên bản lớn:
  không chứa gì riêng của một bản cài. Server công bố mình hỗ trợ gì (phiên bản API, module đang bật, endpoint OAuth
  theo RFC 8414) để mini app tự dò thay vì viết cứng.
- **Giao diện mini app trong sản phẩm của người vận hành:** core vẫn chỉ có API (luật kiến trúc số 9), nên phần này
  là **hợp đồng cho giao diện chủ** — giao diện của người vận hành, ở repo riêng — chứ không phải giao diện trong
  repo này: danh sách mini app khách đã cài kèm chỗ mở; token ngắn hạn cấp cho giao diện mini app theo phiên của
  người dùng đang đăng nhập, chỉ trong quyền khách đã đồng ý và trong quyền của chính người dùng đó; cách ly giữa
  mini app và trang chủ (mini app không đọc được token hay dữ liệu của trang chủ). Kèm SDK TypeScript cho giao diện
  mini app: người dùng hiện tại, gọi API, yêu cầu thanh toán, gửi thông báo — mỗi hàm là một lời gọi API có sẵn,
  không có đường tắt nào vượt quyền.
- **Hướng dẫn cho nhà phát triển mini app** (EN + VI) và một mini app mẫu ở repo riêng; CI cài mini app mẫu vào một
  server thật và chạy nó trên cả hai database, như `examples/external-module/` làm cho module.

Hai kiểu vận hành đều phải chạy được, không mục nào giả định một kiểu: một công ty tự vận hành nền của họ (chợ ứng
dụng của riêng họ, tự duyệt) và một dịch vụ phục vụ nhiều khách. Core không phụ thuộc một danh bạ mini app trung
tâm; danh bạ dùng chung giữa các bản cài, nếu có, là dịch vụ tuỳ chọn ngoài core.

Pháp lý cần đối chiếu khi làm: mini app nhận dữ liệu cá nhân của người dùng thì người vận hành và nhà phát triển
là hai bên xử lý dữ liệu (bảo vệ dữ liệu cá nhân — văn bản và điều khoản ghi trong quyết định của phần này, cần đối
chiếu văn bản gốc / người có chuyên môn pháp lý); thanh toán trong mini app vẫn đi qua đối tác có giấy phép (D-187).

## Thứ tự làm

1. Máy chủ ủy quyền OAuth 2.0 trong `auth` (PKCE, refresh token xoay vòng, thu hồi, kiểm tra, siêu dữ liệu).
2. Ứng dụng, duyệt, cài / đồng ý / gỡ, webhook theo ứng dụng, chặn lạm dụng, audit.
3. Ứng dụng trả phí (D-232): giá có ngày hiệu lực, thuê bao ứng dụng và dùng thử, `AddonBilling` ở kernel + phần
   `plans` gọi nó (kỳ `addons` cho gói theo năm), hồ sơ nhà phát triển + KYC + thoả thuận, tỉ lệ chia, bút toán,
   thời gian hoàn tiền, chi trả qua `Ledger::create_payout`, đổi giá báo trước.
4. Mini app (D-20261125-miniapp-platform): bản mô tả và tự dò; hợp đồng cho giao diện chủ, token theo phiên người
   dùng, SDK cho giao diện mini app; hướng dẫn và mini app mẫu có CI.

## Chỗ dễ sai

- Tính phần nền tảng bằng một phép làm tròn riêng — hai phần lệch một đồng so với khoản phí; phần nền tảng phải
  là phần còn lại (D-232).
- Áp giá mới ngay khi nhà phát triển lưu, hay giữa kỳ đang chạy; tự tính tiền khách đang dùng khi ứng dụng chuyển
  từ miễn phí sang có phí (D-232).
- Chi cho nhà phát triển khi KYC đã hết hạn hay thoả thuận có phiên bản mới chưa chấp nhận — kiểm lại điều kiện
  ngay trong job chi, không tin trạng thái lưu sẵn (D-232).
- Gọi `PayoutGateway` trong transaction gom khoản phí; gom bằng đọc rồi ghi — hai lần chạy chi hai lần (D-232).
- `apps` phụ thuộc crate `plans` (đăng ký sau) — đúng là `plans` đọc điểm cắm `AddonBilling` của kernel (D-232).
- Nền tảng giữ tiền hộ nhà phát triển như ví — phần chia là khoản phải trả từ doanh thu của chính nền tảng, chi
  theo kỳ, không có số dư rút tuỳ ý (D-187, D-232).

## Căn cứ pháp lý và chuẩn

| Nội dung | Văn bản / chuẩn | Ghi chú |
|---|---|---|
| Chuyển dữ liệu cá nhân cho bên thứ ba (ứng dụng) cần thông báo / đồng ý, ghi nhận | Luật 91/2025/QH15; NĐ 356/2025 | Cần đối chiếu: khách là bên kiểm soát, nhà phát triển là bên xử lý |
| OAuth 2.0 / 2.1, PKCE, thu hồi, kiểm tra token, siêu dữ liệu máy chủ | RFC 6749, 7636, 7009, 7662, 8414, 8707; bản nháp OAuth 2.1 | Theo bản ổn định tại lúc làm |
| Nền tảng là bên bán: hoá đơn điện tử cho khách về phí ứng dụng | NĐ 123/2020/NĐ-CP, sửa bởi NĐ 70/2025/NĐ-CP | Cần đối chiếu văn bản gốc (D-232) |
| Tiền đi qua đối tác có giấy phép; nền tảng không giữ tiền của khách | NĐ 52/2024/NĐ-CP (thanh toán không dùng tiền mặt); D-187 | Cần đối chiếu (D-232) |
| Khấu trừ thay khi chi cho hộ / cá nhân nhà phát triển | NĐ 117/2025/NĐ-CP; Luật Thuế TNCN | Cần đối chiếu: có áp cho chia doanh thu ứng dụng không (D-232) |
| Thuế nhà thầu nước ngoài khi chi cho nhà phát triển ngoài Việt Nam | Luật Quản lý thuế 38/2019/QH14; TT 103/2014/TT-BTC (và văn bản thay thế) | Cần đối chiếu; tới khi đó chỉ mở cho nhà phát triển trong nước (D-232) |
| Báo trước khi đổi giá dịch vụ thuê bao, thông tin trung thực cho người mua | Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 | Số ngày báo trước là cấu hình — cần đối chiếu (D-232) |
| Thoả thuận nhà phát triển có phiên bản, bằng chứng chấp nhận | Luật Giao dịch điện tử 20/2023/QH15 | Dùng văn bản có phiên bản của 37 (D-232) |

## Agent làm bước này tự quyết

Thời hạn token; thư viện OAuth (tự viết theo RFC hay crate có sẵn — tra `check-library-api`, ghi `STACK.md`); tiêu
chí duyệt ứng dụng. Tỉ lệ chia mặc định của nền tảng; giảm giá có cần báo trước như tăng giá không; có ngưỡng chi
tối thiểu không (D-232). Ghi vào DECISIONS.
