# Bước 49 — Tiếp thị liên kết và bán qua livestream (`affiliate`)

> Lộ trình: `docs/ROADMAP.md` bước 49 · Thiết kế: `docs/research/business-foundation.md` mục 3 (trụ 4,
> trụ 5), mục 9 · Quyết định: D-186, D-187, D-188, D-190, D-194 · Đặc tả liên quan:
> `docs/steps/45-orders.md` (giỏ mang `attribution`, event `orders.*`)

## Mục tiêu

Cửa hàng, chuỗi và sàn bán qua người tiếp thị liên kết (affiliate, KOL / KOC) và qua livestream, đúng
luật: người tiếp thị và người livestream **phải xác thực danh tính** trước khi hoạt động (Luật TMĐT
2025, cổng chặn của bước 37). Xong bước này: người tiếp thị đăng ký có KYC, có link và mã riêng; đơn
được ghi nhận theo link / mã / phiên livestream trong thời hạn ghi nhận; hoa hồng theo sản phẩm / danh
mục là **dữ liệu có ngày hiệu lực**; hoa hồng ghi sổ cái khi đơn hoàn tất, trừ lại khi trả hàng; chi
trả qua `PayoutGateway` có **khấu trừ thuế thu nhập cá nhân** theo hồ sơ thuế (tỉ lệ và ngưỡng là dữ
liệu của `tax`) và chứng từ khấu trừ cho người nhận; phiên livestream có người phát đã xác thực, sản
phẩm gắn trong phiên, đơn ghi nhận theo phiên, lưu vết phiên.

Tiền hoa hồng là tiền của chính khách (người bán) trả cho người tiếp thị — không phải tiền của người
mua giữ hộ; vẫn chỉ đi qua `PayoutGateway` hay chuyển khoản do khách tự làm rồi ghi nhận (D-187).

## Bắt đầu khi

- Bước 45 (`orders`) đã merge: `orders_carts.attribution` / `PUT /carts/{id}/attribution`,
  `OrderSummary.attribution`, event `orders.order_placed` / `order_cancelled` / `order_completed` /
  `refund_completed`.
- Bước 37 đã merge: KYC + cổng chặn (mục đích `affiliate` và `livestream`), xác thực tài khoản theo
  NĐ 147/2024, văn bản pháp lý có phiên bản (`affiliate.program_terms`), sổ căn cứ xử lý dữ liệu.
- Sẵn từ trước: 33 (kho secret: số tài khoản nhận tiền), 34 (người mua theo khách, nhân sự), 35 (tính
  năng / hạn mức), 36 (`Ledger`, `TaxEngine` khấu trừ thay, `PayoutGateway`, `Invoices` cho chứng từ
  khấu trừ), 41 (`catalog`).
- Song song được với 46, 47, 48.

## Phạm vi

Crate mới `modules/affiliate` (`autoxide-affiliate`), đăng ký sau `orders` (và sau `marketplace` nếu
có — hoa hồng trên sàn tính theo đơn của từng người bán). Bố cục: `partners/`, `tracking/` (link, mã,
lượt bấm, ghi nhận đơn), `commissions/` (quy tắc, tính, sổ cái), `payouts/` (gom, khấu trừ thuế, chi,
chứng từ), `live/` (phiên, sản phẩm gắn, nhật ký), cùng các file chuẩn (`lib.rs`, `api.rs`,
`config.rs`, `permissions.rs`, `events.rs`, `entity/`, `migration/`, `http/`, `tools.rs`,
`privacy.rs`, `tests/`).

Sửa ngoài crate: `Cargo.toml`, `crates/app/Cargo.toml`, `crates/app/src/modules.rs`,
`docs/openapi.json`, `.env.example`, `CHANGELOG.md`, `docs/ROADMAP.md`.

NGOÀI phạm vi: phát video (Autoxide không làm máy chủ phát; phiên trỏ tới nền tảng phát — của khách
hay mạng xã hội); bộ nối lấy số liệu từ TikTok / Facebook (module riêng, cần tài khoản đối tác);
mạng tiếp thị nhiều cấp (chỉ một cấp — hoa hồng nhiều tầng dễ thành kinh doanh đa cấp, có luật riêng —
không làm khi chủ dự án chưa quyết).

## Thiết kế

### Người tiếp thị

Một người tiếp thị (`affiliate_partners`) là một tài khoản trong khách (người mua của khách hay nhân
sự, bước 34) với vai `affiliate`, `livestreamer` hay cả hai. Vòng đời: `pending_verification` (chờ
KYC đúng mục đích + chấp nhận điều khoản chương trình đúng phiên bản + xác thực SĐT) → `pending_review`
(khách duyệt, tuỳ chương trình — `auto_approve`) → `active` ↔ `suspended` → `terminated`. Một hàm duy
nhất `PartnerGate::ensure_active(tenant, partner, purpose)` kiểm cả trạng thái và KYC còn hạn mỗi khi
tạo link, ghi nhận đơn, mở phiên, chi tiền. KYC hết hạn / điều khoản đổi phiên bản (event của
`compliance`) → `suspended`; hoa hồng đã duyệt vẫn được chi khi xác thực lại (không mất tiền đã kiếm).

Hồ sơ thuế của người nhận (`tax_subject`): `individual_resident`, `individual_non_resident`,
`household_business`, `enterprise`; mã số thuế; cam kết thu nhập dưới ngưỡng cả năm (mẫu cam kết của
cơ quan thuế — năm áp dụng, file) nếu người nhận nộp; tài khoản nhận tiền trong kho secret.

### Ghi nhận đơn

- **Link**: `affiliate_links` (mã ngắn duy nhất trong khách, đích: sản phẩm / danh mục / cửa hàng /
  bất kỳ). Giao diện gọi `POST /public/clicks {code}` khi người mua tới qua link → `click_id` + hạn
  (`attribution_days` của chương trình, chụp vào lượt bấm); giao diện giữ `click_id` (cookie phía client)
  và gửi vào giỏ qua `PUT /orders/carts/{id}/attribution`.
- **Mã**: người mua nhập mã người tiếp thị ở giỏ (`attribution.code`).
- **Livestream**: giỏ mang `live_session_id` khi mua từ phiên.
- Khi nhận `orders.order_placed` có `attribution`: chọn một nguồn theo thứ tự ưu tiên là dữ liệu của
  chương trình (mặc định: phiên livestream đang phát / vừa kết thúc trong `live_grace_minutes` và dòng
  hàng có gắn trong phiên → lượt bấm còn hạn → mã); loại nếu người tiếp thị không `active`, nếu người
  mua chính là người tiếp thị (tự mua), nếu lượt bấm hết hạn. Ghi `affiliate_attributions` (một dòng
  mỗi đơn, unique) — `orders` không cần biết `affiliate` có hay không.
- Không ghi nhận chéo khách: `click_id`, mã, phiên đều tra trong `tenant_id` của đơn.

### Hoa hồng

- Quy tắc (`affiliate_commission_rules`, chỉ thêm, có `effective_from` / `effective_to`, `priority`):
  phạm vi `product` / `category` / `default`, áp cho mọi người tiếp thị hay một hạng / một người,
  `rate_bp` và / hoặc `fixed_*`, trần mỗi dòng; cơ sở tính (`net_excl_tax` mặc định — net sau giảm,
  chưa thuế, không gồm phí giao; hay `net_incl_tax`) là cài đặt của chương trình. Quy tắc tra theo
  `placed_at` của đơn; tỉ lệ được **chụp** vào dòng hoa hồng.
- Vòng đời dòng hoa hồng (`affiliate_commissions`, một dòng mỗi dòng đơn): `pending` (đơn đặt) →
  `void` (đơn huỷ) | `approved` (nhận `orders.order_completed` — đã qua hạn đổi trả) → `paid`.
  Hoàn tiền trước khi duyệt: tính lại cơ sở theo số còn lại. Hoàn tiền sau khi duyệt / đã chi: một
  dòng điều chỉnh âm (`affiliate_adjustments`) trừ vào lần chi sau — không sửa dòng đã duyệt.
- Sổ cái (`reference` = `affiliate:commission:<id>` / `affiliate:adjustment:<id>`, qua job enqueue
  cùng transaction):

| Việc | Nợ | Có |
|---|---|---|
| Duyệt hoa hồng | `affiliate_commission_expense` | `affiliate_payable:<partner>` |
| Trừ lại khi trả hàng | `affiliate_payable:<partner>` | `affiliate_commission_expense` |
| Chi trả (gồm khấu trừ) | `affiliate_payable:<partner>` (gross) | `pit_withholding_payable` (thuế) · `payout_clearing:<gateway>` (thực nhận) |
| Lệnh chi thành công | `payout_clearing:<gateway>` | `bank:<org>` (hay tài khoản đối tác chi) |

Trên sàn (bước 46): hoa hồng thuộc về người bán có đơn — `org_id` của bút toán là nút người bán; quy
tắc có thể do sàn đặt (áp mọi người bán) hay do người bán đặt cho sản phẩm của mình; sàn trừ hoa hồng
vào quyết toán của người bán bằng một khoản trong `marketplace` (agent bước 46 / 49 thống nhất bằng
event `affiliate.commission_approved` mang `seller_org_id`).

### Chi trả và khấu trừ thuế

Job `affiliate.build_payouts` theo lịch của chương trình (mặc định tháng): mỗi người tiếp thị gom hoa
hồng `approved` chưa chi + điều chỉnh âm; dưới `min_payout` thì để kỳ sau. Khấu trừ:
`TaxEngine::withholding(tenant, WithholdingRequest { kind: PersonalIncome { payee: tax_subject,
has_commitment }, gross, date: ngày chi })` — `tax` (gói dữ liệu "vn") quyết tỉ lệ (vd 10% với cá nhân
cư trú không ký hợp đồng lao động, 20% với cá nhân không cư trú) và ngưỡng mỗi lần chi; `affiliate`
**không viết số nào trong code**. Doanh nghiệp / hộ kinh doanh có hoá đơn: không khấu trừ TNCN, ghi số
hoá đơn người nhận xuất. Gộp nhiều lần chi nhỏ trong cùng ngày để né ngưỡng là trái luật — job gom
**một lệnh mỗi kỳ** cho mỗi người, không tách nhỏ (D-186: không làm tính năng giúp né luật).

Chi: `PayoutGateway::payout(reference = affiliate:payout:<id>)` ngoài transaction, hoặc chế độ `manual`
(khách tự chuyển khoản rồi ghi `bank_reference`). Thành công → chứng từ khấu trừ thuế TNCN điện tử qua
`Invoices` (giả định bước 36 có loại chứng từ `pit_withholding_certificate`) gửi người nhận; cuối năm
`GET /partners/{id}/tax-statement?year=` tổng hợp thu nhập, thuế đã khấu trừ.

### Phiên livestream

- Mở phiên: người phát là người tiếp thị vai `livestreamer` **đã qua `PartnerGate` với mục đích
  `livestream`** và tài khoản đã xác thực SĐT (NĐ 147/2024); hoặc nhân sự của chính cửa hàng, cũng phải
  qua KYC mục đích `livestream` của bước 37. Khách bật tính năng `affiliate.livestream`.
- Phiên (`affiliate_live_sessions`): cửa hàng / người bán (`org_id`), người phát, nền tảng (`own`,
  `facebook`, `tiktok`, `youtube`, `zalo`, `other`) + URL, tiêu đề, giờ dự kiến, trạng thái
  `scheduled → live → ended`.
- Gắn sản phẩm (`affiliate_live_products`): chỉ biến thể đang bán (`Catalog::snapshot`); lưu giá đang
  niêm yết lúc gắn và chương trình khuyến mại gắn kèm (ID của `promotions`) — giá người mua trả vẫn do
  `orders` tính ở máy chủ. Gỡ / gắn lại là dòng mới (chỉ thêm).
- Nhật ký phiên (`affiliate_live_log`, chỉ thêm): bắt đầu, gắn / gỡ sản phẩm, giá công bố, kết thúc,
  báo vi phạm (từ `compliance`), mỗi dòng có thời điểm và người làm; giữ `LIVE_LOG_RETENTION_YEARS`
  năm. Không lưu video (nếu cần lưu bản ghi hình, lưu link / khoá file do người phát cung cấp).
- Báo cáo phiên: số đơn, doanh thu, hoa hồng, sản phẩm bán chạy theo phiên.

### Kiểu và trait (phác thảo — tra API thật bằng skill `check-library-api`)

```rust
// affiliate::api
pub enum PartnerStatus { PendingVerification, PendingReview, Active, Suspended, Terminated }
pub struct PartnerSummary { pub id: Uuid, pub user_id: Uuid, pub roles: Vec<PartnerRole>, pub status: PartnerStatus, pub code: String } // #[non_exhaustive]
#[async_trait] pub trait Affiliates: Send + Sync {
    async fn find_partner(&self, tenant: TenantId, id: Uuid) -> Result<Option<PartnerSummary>, Error>;
    /// Phiên đang phát của một cửa hàng (cho giao diện / `channels`).
    async fn live_sessions(&self, tenant: TenantId, org: OrgId) -> Result<Vec<LiveSessionSummary>, Error>;
}
```

Giả định: `compliance::api::Kyc::status(tenant, KycSubject::User(user), KycPurpose::{Affiliate, Livestream})`,
`AccountVerification::is_verified`, `LegalDocuments::has_accepted(…, "affiliate.program_terms", version)`;
`tax::api::TaxEngine::withholding` có loại thu nhập cá nhân; `invoices::api::Invoices::issue` có chứng từ
khấu trừ; kernel `PayoutGateway`; `orders::api::Attribution { click_id, code, live_session_id }`.

### Bảng dữ liệu

Quy ước như bước 45 (`tenant_id` đầu mọi khoá; UUIDv7; UTC; `Money` hai cột; JSON trong `text`;
migration có `down`; `outbox_migration`, `jobs_migration`).

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `affiliate_programs` | `tenant_id`, `org_id` (NULL = cả khách), `attribution_days` INT, `live_grace_minutes` INT, `precedence` text, `commission_base`, `auto_approve` BOOL, `payout_schedule`, `min_payout_*`, `payout_mode` (`gateway/manual`), `terms_document_kind`, `updated_at` | pk (`tenant_id`, `org_key`) — `org_key` = `org_id` hay `-` khi NULL (khoá chính không nhận NULL) |
| `affiliate_partners` | `id`, `tenant_id`, `user_id`, `roles` text, `status`, `status_reason` NULL, `code`, `tier` NULL, `tax_subject`, `tax_code` NULL, `commitment_year` NULL, `commitment_file_key` NULL, `payout_account_secret` NULL, `created_at`, `updated_at`, `activated_at` NULL | unique (`tenant_id`, `user_id`); unique (`tenant_id`, `code`); index (`tenant_id`, `status`) |
| `affiliate_links` | `id`, `tenant_id`, `partner_id`, `code`, `target_kind` (`product/category/store/any`), `target_id` NULL, `landing_path`, `created_at`, `disabled_at` NULL | unique (`tenant_id`, `code`); index (`tenant_id`, `partner_id`, `id`) |
| `affiliate_clicks` | `id`, `tenant_id`, `link_id`, `partner_id`, `ip_hash` (`autoxide_kernel::ip_hash`), `user_agent_family` NULL, `created_at`, `expires_at` | index (`tenant_id`, `partner_id`, `created_at`), (`tenant_id`, `expires_at`) — xoá sau `CLICK_RETENTION_DAYS` |
| `affiliate_attributions` | `tenant_id`, `order_id`, `partner_id`, `source` (`live/click/code`), `click_id` NULL, `live_session_id` NULL, `seller_org_id`, `placed_at`, `created_at` | pk (`tenant_id`, `order_id`); index (`tenant_id`, `partner_id`, `placed_at`), (`tenant_id`, `live_session_id`) |
| `affiliate_commission_rules` | `id`, `tenant_id`, `org_id` NULL, `scope` (`product/category/default`), `target_id` NULL, `partner_id` NULL, `tier` NULL, `rate_bp` INT, `fixed_*` NULL, `cap_*` NULL, `priority` INT, `effective_from`, `effective_to` NULL, `created_by`, `created_at` | index (`tenant_id`, `scope`, `target_id`, `effective_from`) — chỉ thêm |
| `affiliate_commissions` | `id`, `tenant_id`, `order_id`, `order_line_id`, `partner_id`, `seller_org_id`, `rule_id`, `rate_bp`, `base_*`, `amount_*`, `status` (`pending/void/approved/paid`), `approved_at` NULL, `payout_id` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `order_line_id`); index (`tenant_id`, `partner_id`, `status`), (`tenant_id`, `order_id`) |
| `affiliate_adjustments` | `id`, `tenant_id`, `partner_id`, `commission_id` NULL, `amount_*` (âm), `reason`, `source_ref`, `payout_id` NULL, `created_at` | unique (`tenant_id`, `source_ref`); index (`tenant_id`, `partner_id`, `payout_id`) |
| `affiliate_payouts` | `id`, `tenant_id`, `partner_id`, `period_from`, `period_to`, `gross_*`, `withheld_*`, `withholding` text (dòng thuế, tỉ lệ, ngưỡng ảnh chụp), `net_*`, `mode`, `status` (`building/sent/paid/failed/cancelled`), `reference`, `external_id` NULL, `bank_reference` NULL, `certificate_id` NULL, `created_at`, `updated_at` | unique (`tenant_id`, `reference`); index (`tenant_id`, `partner_id`, `id`), (`tenant_id`, `status`) |
| `affiliate_live_sessions` | `id`, `tenant_id`, `org_id`, `host_partner_id` NULL, `host_user_id`, `platform`, `stream_url` NULL, `title`, `scheduled_at` NULL, `started_at` NULL, `ended_at` NULL, `status`, `created_at`, `updated_at` | index (`tenant_id`, `org_id`, `status`, `id`), (`tenant_id`, `host_user_id`) |
| `affiliate_live_products` | `id`, `tenant_id`, `session_id`, `variant_id`, `product_id`, `price_shown_*`, `promotion_id` NULL, `pinned_at`, `unpinned_at` NULL | index (`tenant_id`, `session_id`, `variant_id`) |
| `affiliate_live_log` | `id`, `tenant_id`, `session_id`, `kind` (`started/pinned/unpinned/price_announced/ended/reported`), `details` text, `actor_id` NULL, `at` | index (`tenant_id`, `session_id`, `id`) — chỉ thêm |

### API REST (`/api/v1/affiliate`)

| Method + path | Permission | Ghi chú |
|---|---|---|
| `POST /partners` | `affiliate.join` | đăng ký; trả các bước còn thiếu (KYC, điều khoản, SĐT) |
| `GET /partners/me` · `PUT /partners/me` | `affiliate.partner` | hồ sơ, hồ sơ thuế, tài khoản nhận tiền (kho secret), cam kết thu nhập |
| `GET/POST /partners/me/links` · `DELETE /partners/me/links/{id}` | `affiliate.partner` | qua `PartnerGate` |
| `GET /partners/me/stats?from&to` · `GET /partners/me/commissions` · `GET /partners/me/payouts` | `affiliate.partner` | chỉ của mình (phạm vi `Own`) |
| `GET /partners/me/tax-statement?year=` | `affiliate.partner` | tổng thu nhập, thuế đã khấu trừ, chứng từ |
| `GET /partners` · `GET /partners/{id}` · `POST /partners/{id}/transitions` | `affiliate.read` / `affiliate.manage` | duyệt / tạm khoá / chấm dứt có lý do, `If-Match` |
| `GET/PUT /programs/{org_id}` | `affiliate.manage` | cấu hình chương trình |
| `GET /commission-rules` · `POST /commission-rules` · `POST /commission-rules/{id}/close` · `GET /commission-rules/preview` | `affiliate.manage` | dữ liệu có ngày hiệu lực; xem trước hoa hồng cho một dòng giả định |
| `GET /commissions` · `GET /payouts` · `POST /payouts/{id}/mark-paid` · `POST /payouts/{id}/retry` | `affiliate.finance` | `mark-paid` cho chế độ `manual`, có `bank_reference` (D-184) |
| `POST /public/clicks` | công khai (rate limit) | `{code}` → `{click_id, expires_at}`; mã sai → 404 |
| `GET /public/live-sessions/{id}` | công khai | phiên + sản phẩm đang gắn (giá hiển thị lấy lại từ `catalog` lúc đọc) |
| `POST /live-sessions` · `PUT /live-sessions/{id}` · `POST /live-sessions/{id}/start` · `/end` | `affiliate.live.host` (người phát) / `affiliate.live.manage` | `start` qua `PartnerGate(livestream)` |
| `POST /live-sessions/{id}/products` · `DELETE /live-sessions/{id}/products/{variant_id}` | `affiliate.live.host` / `affiliate.live.manage` | gắn / gỡ |
| `GET /live-sessions` · `GET /live-sessions/{id}/log` · `GET /live-sessions/{id}/report` | `affiliate.live.manage` / người phát (của mình) | lưu vết, báo cáo |

### Tool MCP

`list_partners` (`read_only`), `decide_partner`, `list_commission_rules` (`read_only`),
`add_commission_rule`, `preview_commission` (`read_only`), `partner_stats` (`read_only`),
`list_payouts` (`read_only`), `live_session_report` (`read_only`). Không có tool chi tiền hay đánh dấu
đã chi (việc có tiền cần người duyệt qua REST).

### Event

`affiliate.partner_registered`, `affiliate.partner_status_changed`, `affiliate.link_created`,
`affiliate.order_attributed`, `affiliate.commission_approved` (mang `seller_org_id` cho bước 46),
`affiliate.commission_voided`, `affiliate.commission_adjusted`, `affiliate.payout_sent`,
`affiliate.payout_paid`, `affiliate.payout_failed`, `affiliate.live_session_started`,
`affiliate.live_session_ended`, `affiliate.live_product_pinned`. Không có event cho mỗi lượt bấm (số đo
`affiliate_clicks_total` không nhãn ID).

Nghe: `orders.order_placed`, `orders.order_cancelled`, `orders.order_completed`,
`orders.refund_completed`, sự kiện KYC / điều khoản / báo vi phạm của `compliance`,
`privacy::USER_DELETED`. Chịu nhận lại (khoá theo `order_id`, `order_line_id`, `source_ref`).

### Job và lịch

`affiliate.build_payouts` (theo lịch chương trình), `affiliate.send_payout` (enqueue),
`affiliate.poll_payouts` (mỗi 15 phút), `affiliate.issue_certificate` (enqueue khi `paid`),
`affiliate.auto_end_sessions` (mỗi 10 phút: phiên `live` quá `LIVE_MAX_HOURS` → `ended`),
`affiliate.purge_clicks` (mỗi ngày), `affiliate.post_ledger` (enqueue cùng transaction).

### Cấu hình

`AUTOXIDE_AFFILIATE_CLICK_RETENTION_DAYS` (90), `AUTOXIDE_AFFILIATE_LIVE_MAX_HOURS` (12),
`AUTOXIDE_AFFILIATE_LIVE_LOG_RETENTION_YEARS` (mặc định 5 — cần đối chiếu, xem "Căn cứ pháp lý"),
`AUTOXIDE_AFFILIATE_CLICKS_PER_MINUTE` (giới hạn mỗi địa chỉ cho `POST /public/clicks`). Chương trình,
quy tắc, lịch chi là dữ liệu theo khách. Tính năng: `affiliate.links`, `affiliate.livestream`. Hạn
mức: `affiliate.partners`.

### Permission

`affiliate.join`, `affiliate.partner`, `affiliate.read`, `affiliate.manage`, `affiliate.finance`,
`affiliate.live.host`, `affiliate.live.manage`.

### Mã lỗi

| Code | HTTP | Tiếng Việt |
|---|---|---|
| `affiliate.partner_not_found` | 404 | Không tìm thấy người tiếp thị liên kết. |
| `affiliate.already_registered` | 409 | Tài khoản này đã đăng ký tiếp thị liên kết. |
| `affiliate.kyc_required` | 403 | Cần xác thực danh tính trước khi {purpose}. |
| `affiliate.phone_verification_required` | 403 | Cần xác thực số điện thoại trước khi livestream. |
| `affiliate.terms_not_accepted` | 403 | Cần đồng ý điều khoản chương trình phiên bản {version}. |
| `affiliate.partner_not_active` | 409 | Tài khoản tiếp thị liên kết đang ở trạng thái {status}. |
| `affiliate.invalid_transition` | 409 | Không chuyển từ {from} sang {to} được. |
| `affiliate.code_taken` | 409 | Mã {code} đã có người dùng. |
| `affiliate.link_not_found` | 404 | Link tiếp thị không tồn tại hoặc đã tắt. |
| `affiliate.rule_overlap` | 409 | Đã có quy tắc cùng phạm vi hiệu lực từ {from}. |
| `affiliate.rule_in_past` | 422 | Không thêm quy tắc có hiệu lực trong quá khứ. |
| `affiliate.payout_account_missing` | 422 | Chưa khai tài khoản nhận tiền. |
| `affiliate.tax_profile_incomplete` | 422 | Hồ sơ thuế còn thiếu: {fields}. |
| `affiliate.payout_not_retryable` | 409 | Lệnh chi đang ở trạng thái {status}, không chi lại được. |
| `affiliate.session_not_found` | 404 | Không tìm thấy phiên livestream. |
| `affiliate.session_not_live` | 409 | Phiên livestream chưa bắt đầu hoặc đã kết thúc. |
| `affiliate.variant_not_sellable` | 422 | Sản phẩm {sku} không đang bán, không gắn vào phiên được. |
| `affiliate.clicks_throttled` | 429 | Quá nhiều lượt; thử lại sau {seconds} giây. |

## Thứ tự làm

1. Khung crate, permission, cấu hình, `api.rs`, đăng ký. → chuẩn bị
2. Người tiếp thị: đăng ký, `PartnerGate` (KYC, điều khoản, SĐT), hồ sơ thuế, tài khoản nhận tiền; link,
   mã, lượt bấm, ghi nhận đơn; quy tắc hoa hồng có ngày hiệu lực. → **mục 1**
3. Hoa hồng: vòng đời, sổ cái, trừ lại khi trả hàng. → **mục 2**
4. Chi trả: gom kỳ, khấu trừ TNCN qua `TaxEngine`, `PayoutGateway` / thủ công, chứng từ khấu trừ, tổng
   hợp năm. → **mục 2**
5. Livestream: phiên, gắn sản phẩm, ghi nhận theo phiên, nhật ký, báo cáo. → **mục 3**
6. `PersonalData`, `USER_DELETED`, tool MCP, `docs/openapi.json`, `.env.example`, CHANGELOG, ROADMAP,
   quyết định.

## Test bắt buộc

Cả SQLite và Postgres; `TestApp` với `orders` và module nền thật; `IdentityVerifier`, `PayoutGateway`,
`EInvoiceProvider` giả.

- `unverified_partner_cannot_create_links_or_host_live`; `kyc_expiry_suspends_partner_keeps_approved_commissions`.
- `click_attribution_within_window_only`; `self_purchase_not_attributed`;
  `live_session_beats_click_by_default_precedence`; `attribution_not_cross_tenant`.
- `order_placed_redelivered_attributes_once` — event giao hai lần song song: một attribution, một dòng
  hoa hồng mỗi dòng đơn.
- `commission_rate_snapshot_uses_rule_effective_at_placed_at`.
- `commission_approved_only_on_completion_voided_on_cancel`.
- `refund_after_approval_creates_negative_adjustment_and_ledger_reversal`.
- `payout_withholding_follows_tax_data` — đổi ngưỡng / tỉ lệ trong dữ liệu `tax` (2 triệu → 5 triệu):
  cùng số tiền, khấu trừ khác nhau theo ngày chi; doanh nghiệp không bị khấu trừ.
- `one_payout_per_partner_per_period` — hai worker gom cùng lúc: mỗi hoa hồng vào đúng một lệnh chi.
- `payout_ledger_balances` — Σ nợ = Σ có; `affiliate_payable` còn lại = hoa hồng duyệt chưa chi.
- `duplicate_payout_webhook_marks_paid_once`; `certificate_issued_once_per_payout`.
- `live_log_is_append_only_and_records_pins_and_prices`; `auto_end_long_sessions`.
- `public_clicks_are_rate_limited`.
- `user_deleted_keeps_tax_and_ledger_records` — xoá tài khoản: link, lượt bấm xoá; hồ sơ thuế và chứng
  từ đã khấu trừ giữ theo luật kế toán / thuế.
- Rò rỉ: `tenant_isolation`; `partner_sees_only_own_commissions`; `seller_sees_only_own_affiliate_costs`
  (trên sàn).

## Tiêu chí xong

- [ ] Ba mục của bước 49 có commit và test; gate đầy đủ xanh trên cả hai database.
- [ ] Không số thuế nào viết cứng trong `affiliate`; tỉ lệ và ngưỡng đọc từ dữ liệu "vn" của `tax`.
- [ ] Người tiếp thị / người phát chưa xác thực không làm được gì có tác dụng (link, ghi nhận, phiên, chi).
- [ ] Quyết định trong `docs/DECISIONS.md`: thứ tự ưu tiên ghi nhận, cơ sở tính hoa hồng, bảng bút toán,
      khấu trừ TNCN kèm căn cứ và "cần đối chiếu", lưu vết livestream, không làm hoa hồng nhiều tầng.
- [ ] `docs/openapi.json`, `.env.example`, CHANGELOG, ROADMAP; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Khấu trừ theo lần chi, không theo dòng hoa hồng**: ngưỡng áp cho mỗi lần trả thu nhập; tính trên
  tổng của lệnh chi (sau điều chỉnh âm), lưu ảnh chụp tỉ lệ / ngưỡng trên lệnh chi.
- **Lệnh chi âm**: điều chỉnh âm lớn hơn hoa hồng của kỳ → không chi, mang sang kỳ sau; không đòi tiền
  người tiếp thị tự động.
- **Không gọi `PayoutGateway` / `Invoices` trong transaction**; gửi lại dùng cùng `reference`.
- **Hoa hồng trên đơn tách theo người bán** (sàn): một lần mua tách nhiều đơn → mỗi đơn một attribution,
  cùng nguồn; hoa hồng tính theo từng đơn.
- **Lượt bấm là dữ liệu cá nhân gián tiếp**: chỉ lưu `ip_hash` (của kernel), không lưu IP, không lưu
  chuỗi user agent đầy đủ; có thời hạn xoá.
- **Giờ phiên** lưu UTC; báo cáo theo múi giờ của khách.
- **Phiên kết thúc rồi vẫn có đơn tới** (người mua đặt chậm): cửa sổ `live_grace_minutes`, không hơn.

## Căn cứ pháp lý

- Luật Thương mại điện tử 122/2025/QH15 (từ 01/7/2026): xác thực danh tính người bán, người livestream
  bán hàng, người tiếp thị liên kết; nghĩa vụ của người livestream (thông tin sản phẩm, giá, người bán)
  và lưu thông tin phiên — điều khoản cụ thể và thời hạn lưu trong nghị định hướng dẫn **cần đối chiếu
  văn bản gốc**.
- NĐ 147/2024/NĐ-CP: tài khoản phải xác thực (SĐT / định danh) mới được livestream.
- Thuế TNCN với thu nhập hoa hồng của cá nhân không ký hợp đồng lao động: khấu trừ 10% khi mỗi lần chi
  từ 2.000.000 đồng trở lên (Thông tư 111/2013/TT-BTC, Điều 25) — các nguồn đang nêu NĐ 253/2026 đổi
  ngưỡng lên 5.000.000 đồng / lần; cá nhân không cư trú 20%; cam kết thu nhập dưới ngưỡng cả năm thì
  không khấu trừ — **tất cả là dữ liệu có ngày hiệu lực trong `tax`, cần đối chiếu văn bản gốc trước
  khi chạy thật** (D-186, D-194).
- Chứng từ khấu trừ thuế TNCN điện tử: NĐ 123/2020 sửa bởi NĐ 70/2025 — cần đối chiếu mẫu.
- Luật BVDLCN 91/2025 + NĐ 356/2025: theo dõi lượt bấm, hồ sơ thuế của người tiếp thị là xử lý dữ liệu
  cá nhân — căn cứ xử lý ghi trong sổ của `compliance`, thời hạn lưu.
- Luật Kế toán 88/2015: hoa hồng, chi trả, chứng từ khấu trừ lưu ít nhất 10 năm.

## Agent làm bước này tự quyết

- Thứ tự ưu tiên ghi nhận mặc định (khuyến nghị: livestream → lượt bấm gần nhất → mã) và thời hạn ghi
  nhận mặc định (khuyến nghị 7 ngày) — là dữ liệu chương trình, ghi quyết định.
- Cơ sở tính hoa hồng mặc định (khuyến nghị `net_excl_tax`).
- Chế độ chi mặc định (khuyến nghị `manual` khi khách chưa cấu hình `PayoutGateway`).
- Thời hạn lưu nhật ký phiên mặc định khi nghị định chưa rõ (khuyến nghị 5 năm, là cấu hình; ghi "cần
  đối chiếu").
- Có cho người tiếp thị tự đặt mã riêng không: khuyến nghị có, kiểm trùng và từ cấm (dữ liệu).

## Bổ sung theo đối chiếu thiết kế đợt 2 (D-223…D-227)

> Nguồn: `docs/prompts/ui-phase2-api.md`; màn tương ứng ghi trong `docs/research/ui-traceability.md` (đợt 2).

- **Ngày chuyển khoản.** `mark-paid` nhận `paid_on` (`date`, bắt buộc, không sau hôm nay, không trước ngày tạo kỳ chi)
  cùng `bank_reference`; ngày này là ngày chi trên chứng từ khấu trừ thuế và bút toán. Ngưỡng khấu trừ thuế TNCN
  (2 hay 5 triệu đồng mỗi lần chi) giữ là dữ liệu — **cần đối chiếu văn bản gốc**.
