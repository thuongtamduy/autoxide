# Đặc tả từng bước (30–67)

Mỗi bước của `docs/ROADMAP.md` từ 30 tới 49, và bước 51 tới 67, có một file đặc tả ở đây (bước 50 chỉ là việc của chủ dự án). Đặc tả nói **cách làm**:

- bảng dữ liệu, API, permission, event, mã lỗi;
- thứ tự commit, test bắt buộc, tiêu chí xong;
- chỗ dễ sai, căn cứ pháp lý.

`ROADMAP.md` vẫn là danh sách **việc phải xong**: đánh dấu `[x]` ở đó.

## Cách dùng

1. Đọc `CLAUDE.md`, rồi file này, rồi đặc tả của bước. Thiết kế chung nằm ở
   `docs/research/business-foundation.md`, lý do ở `docs/DECISIONS.md`.
2. Ưu tiên khi có chỗ lệch nhau:

   | Tài liệu | Thắng khi |
   |---|---|
   | `CLAUDE.md` | Luôn đứng trên mọi tài liệu khác |
   | File này | Lệch với một đặc tả |
   | Đặc tả của module **cung cấp** một hợp đồng | Lệch với đặc tả của module **dùng** hợp đồng đó, vd bước 45 giả định chữ ký `api` của bước 41–44: theo 41–44 |
   | Code thật | Lệch với chữ ký phác thảo trong đặc tả: tra bằng skill `check-library-api`, theo code |

3. Phần "Agent làm bước này tự quyết" của mỗi đặc tả: người làm chọn, rồi ghi vào `docs/DECISIONS.md`.
   Không dừng lại hỏi trừ khi `CLAUDE.md` bảo phải hỏi (secret thật, việc không hoàn tác được ở bên ngoài).
4. Thấy đặc tả sai hay thiếu thì sửa đặc tả trong cùng PR, ghi một dòng lý do trong commit.
5. **Một đặc tả, một phiên bản đúng (D-228).** Quyết định mới làm đổi đặc tả thì sửa thẳng vào thân (đúng
   bảng, đúng mục API, test…), xoá câu cũ bị thay, ghi số quyết định trong ngoặc — không nối thêm mục
   "Bổ sung theo …" ở cuối. Đặc tả nào còn mục "Bổ sung" cũ thì commit đầu tiên của bước đó gộp chúng vào
   thân trước khi viết code.

| Bước | Đặc tả | Module / phần chính |
|---|---|---|
| 30 | [30-public-launch.md](30-public-launch.md) | CI, phát hành, file OSS, WebP, deploy không gián đoạn |
| 31 | [31-api-contract.md](31-api-contract.md) | Chặn thay đổi phá vỡ, SDK TypeScript, test nâng cấp |
| 32 | [32-layout-cleanup.md](32-layout-cleanup.md) | Bố cục module và kernel, việc để lại |
| 33 | [33-tenancy.md](33-tenancy.md) | `kernel::tenancy`, mặt điều khiển, `tenant_id` mọi bảng — ba nhóm A/B/C |
| 34 | [34-organizations-identity.md](34-organizations-identity.md) | `address`, `organizations`, quyền theo độ sâu, danh tính hai lớp |
| 35 | [35-plans-subscriptions.md](35-plans-subscriptions.md) | `plans`, quyền dùng, hạn mức, thuê bao |
| 36 | [36-money-ledger-tax.md](36-money-ledger-tax.md) | `tax`, `ledger` (gồm thanh toán), `invoices` |
| 37 | [37-compliance.md](37-compliance.md) | `compliance`, `notifications` |
| 38 | [38-cross-database-reporting.md](38-cross-database-reporting.md) | `reporting`, phân phối cha → con, tách / gộp khách |
| 39 | [39-operations-performance.md](39-operations-performance.md) | Backup, số đo, cảnh báo, đo tải |
| 40 | [40-core-capabilities.md](40-core-capabilities.md) | Header bảo mật, MCP, OIDC, tải thẳng S3, SSE, rate limit |
| 51 | [51-ui-contracts.md](51-ui-contracts.md) | Sổ điều hướng, hộp việc, tìm xuyên phân hệ, số liệu tóm tắt, hoạt động, nhúng bản ghi trong nội dung (D-200); view đã lưu, thao tác hàng loạt, giải thích quyền, lệnh cần duyệt cho agent, ghi chú trên bản ghi (D-201) |
| 52 | [52-admin-experience.md](52-admin-experience.md) | Bàn kỹ thuật, hoàn tác, AI trong quản trị, thông báo, sức khoẻ người bán và tuân thủ (D-201) |
| 53 | [53-support.md](53-support.md) | `support`: phiếu hỗ trợ khách hàng, SLA theo giờ làm, phân công theo nút, chuyển sang khiếu nại (D-204) |
| 54 | [54-authorization.md](54-authorization.md) | Phân quyền chuẩn hàng đầu: hợp đồng quyền trong OpenAPI, deny, rào, chia sẻ bản ghi, step-up, nâng quyền đúng lúc, hai người, quyền theo trường, mô phỏng, rà soát, AuthZEN (D-205) |
| 55 | [55-storefront.md](55-storefront.md) | Banner theo vị trí, bộ sưu tập sản phẩm, thông tin cửa hàng công khai (D-219) |
| 56 | [56-imports.md](56-imports.md) | `imports`: chuyển dữ liệu từ Shopify, WooCommerce, Haravan, Sapo, KiotViet, giữ đường dẫn cũ (D-219) |
| 57 | [57-analytics.md](57-analytics.md) | `analytics`: số liệu truy cập không cookie, phễu, theo dõi theo người khi đồng ý (D-220) |
| 58 | [58-automations.md](58-automations.md) | `automations`: luật "khi … thì …" chạy tool MCP với quyền người tạo, chờ duyệt, chống vòng lặp (D-220) |
| 67 | [67-page-builder.md](67-page-builder.md) | Trang dựng từ khối, AI tạo trang, bộ giao diện website, kiểm tra sức khoẻ website (`content` + sổ khối của kernel, D-229) |
| 59–66 | [59](59-wholesale.md) · [60](60-bookings.md) · [61](61-conversations.md) · [62](62-recurring.md) · [63](63-multi-currency.md) · [64](64-ads.md) · [65](65-apps.md) · [66](66-scim.md) | Bán sỉ B2B, đặt lịch, chat, bán theo kỳ, đa tiền tệ, quảng cáo trong sàn, chợ ứng dụng OAuth, SCIM (D-221) |
| 41–49 | [41](41-catalog.md) · [42](42-inventory.md) · [43](43-promotions.md) · [44](44-shipping.md) · [45](45-orders.md) · [46](46-marketplace.md) · [47](47-customers.md) · [48](48-channels.md) · [49](49-affiliate.md) | Module thương mại |

## Luật chung đã chốt (D-198)

**Ngữ cảnh khách.**
- Hàm của service và trait `api` **chạm database** nhận `tenant: &TenantContext`. `TenantContext` là
  nguồn duy nhất của `Database` (đặc tả 33, mục quyết định 1).
- Chỉ hàm không chạm database (vd mã hoá `seal` / `open`) mới nhận `TenantId`.
- Code hệ thống không có request (job, event) mở ngữ cảnh bằng `DatabaseRegistry` trước khi gọi.
- Chữ ký trong một đặc tả viết `tenant: TenantId` cho hàm chạm database thì đọc là `&TenantContext`.

**Khách mặc định và hằng số.**
- `DEFAULT_TENANT` là UUID cố định: ngoại lệ có chủ đích của luật "UUIDv7 sinh trong ứng dụng".
- `NO_TENANT` cho job / lịch của cả database.
- Header chọn khách: `Autoxide-Tenant`.

**Cây tổ chức ở kernel.**
- Bảng `kernel_org_units` và `kernel_org_closure` thuộc kernel, vì lọc theo nhánh cần subquery trong câu SQL
  của mọi module, mà luật kiến trúc 1 cấm module đọc bảng của module khác.
- Module `organizations` giữ tên, loại, trạng thái, địa chỉ, cài đặt của nút.
- **Nút gốc của khách có ID bằng tenant ID.**

**Phân loại bảng.**
- Mặt điều khiển (không `tenant_id`, dùng chung mọi khách), khai khi tạo bảng:
  - `kernel_tenants`, `kernel_placements`, `kernel_backups`, `kernel_relocations`
  - danh mục đơn vị hành chính của `address`
  - dữ liệu luật dùng chung: `catalog_restriction_rules`, `promotions_legal_limits`, `promotions_notice_rules`
  - trang mẫu dùng chung: dòng `tenant_id` NULL của `content_page_templates` (bước 67)
  - chợ ứng dụng trả phí: bảng `apps_*` về giá, phí, chia doanh thu, chi trả nhà phát triển (bước 65, gom xuyên khách)
  - `invoices_archives` (bản lưu kế toán còn sau khi xoá khách)
- `kernel_sequences` (đánh số chứng từ) là bảng **cấp khách hàng**, có ở mọi database dữ liệu: số phải cấp
  trong cùng transaction với chứng từ.

**Tiền.**
- Thanh toán, webhook của đối tác, đối soát, VietQR, chi trả nằm trong module `ledger`, không có module
  `payments` riêng. Mọi đồng tiền thành bút toán mà không cần transaction xuyên module.
- Chuỗi băm của sổ cái **không băm `tenant_id`**, để bước 38 tách / gộp khách không làm gãy chuỗi.

**Thứ tự đăng ký module** (`crates/app/src/modules.rs`; bên cung cấp trước bên dùng):

```
kernel → address → organizations → auth (và các module sẵn có theo thứ tự hiện tại,
         trừ newsletter, comments)
       → tax → ledger → invoices → compliance → notifications → newsletter, comments, support, analytics,
         bookings, conversations, apps → plans
       → catalog → inventory, promotions, shipping → orders → marketplace → affiliate, customers, channels,
         wholesale, recurring, ads
       → imports → reporting → automations   (cuối cùng: nghe event và gọi tool của mọi module)
```

- `compliance` đứng trước `notifications`, nên `compliance` gửi mail bằng `dyn Mailer` và OTP bằng
  `dyn SmsSender`.
- `plans` đứng sau mọi module nền. Module đăng ký trước nó hỏi quyền dùng qua tay cầm gắn muộn
  `EntitlementsHandle`, như `ResourceDirectory`.

**Điểm cắm cho module đăng ký sau.**
- Module đăng ký trước không `require` được trait của module đăng ký sau.
- Khi module trước cần gọi tới module sau, module trước công bố một "điểm cắm" trong `api` của nó.
  Ví dụ: `OrderExtensions` của `orders` cho `marketplace`, `customers`; `ListingGate` của `catalog` cho
  `marketplace`; `ContentLinks` của `content` cho `catalog`, `promotions` (bước 55).
- Module sau gắn vào điểm cắm lúc `start`. Không có module sau thì điểm cắm rỗng.

**Event giữa module thương mại.**
- Nhận event của module có thể chưa cài thì dùng `Subscription::optional` (làm ở commit chuẩn bị của bước 41).
- Tên event do module **phát** quyết.
- `orders` phát `orders.order_confirmed`, `orders.order_cancelled`, `orders.return_received`,
  `orders.fulfillment_requested`, `orders.return_pickup_requested` (payload `after.shipment` là
  `ShipmentRequest` của `shipping`).
- `shipping` phát `shipping.shipment_status_changed`, `…_delivered`, `…_delivery_failed`, `…_returned`,
  `shipping.cod_collected`… (đặc tả 44).

**Giữ chỗ.**
- Giữ chỗ (tồn kho, lượt khuyến mại, hạn mức) gọi trực tiếp qua `api`, có hạn và mã tham chiếu.
- Xác nhận / trả lại sau khi đơn đã tồn tại thì đi qua event; gọi trực tiếp vẫn được, hai đường dùng chung
  một hàm chịu được gọi lại.

**`unique_key` của job.**
- Khoá còn sống tới khi job đã xong bị dọn, nên job lặp lại theo lịch phải dùng khoá có khe thời gian
  (vd `<tài khoản>:<biến thể>:<giờ>`).
- Khoá cố định sẽ chặn mọi lần chạy sau.

## API mới của kernel mà các bước cần

Theo D-038, mọi hàm mới trên `Module` có cài đặt mặc định. Bước nào cần trước thì làm, kèm test. Bước
sau dùng lại, không làm lại.

| API | Bước dựng | Dùng ở |
|---|---|---|
| `tenancy` (`TenantId`, `OrgId`, `TenantContext`, `DatabaseRegistry`, `ScopeExt`), `Module::upgrades`, `Module::seed_tenant`, mã hoá AEAD (`seal` / `open`), `kernel_tenant_settings` | 33 | mọi bước sau |
| `Scope`, `Permission::scoped`, `Principal::{scope_condition, require_at, require_write_at}`, `OrgTree`, `tenancy::nearest`, `Module::org_settings`, cột tổ chức trên outbox | 34 | 35–49 |
| `entitlement` (`Entitlements`: `module_enabled`, `has_feature`, `reserve` / `confirm` / `release`, `record_usage`, `snapshot`), `EntitlementsHandle`, `Module::{features, limits, always_enabled, sample_data}` | 35 | 41–49 |
| `decimal`, `numbering`, `payment` (`PaymentGateway`, `PayoutGateway`, `BankFeed`, `ExchangeRateSource`), `einvoice`, `StartContext::{contribute, contributions}`, `rows::lock_one_shared`, `Module::facts` | 36 | 38, 41–49 |
| `identity::IdentityVerifier`, `sms::SmsSender`, `StartContext::optional`, `Module::retention` | 37 | 45–49 |
| `tenancy::{distribution, relocate}`, `Module::relocation` | 38 | 41 (phân phối danh mục) |
| `JobRegistration::timeout` + gia hạn lease cho job chạy lâu | 39 | backup, xuất lớn |
| `Module::{description, mcp_resources, prompts}`, `autoxide_kernel::net` (chặn SSRF, dời từ `webhooks`) | 40 | OIDC, MCP |
| `Subscription::optional`, `autoxide_kernel::csv` (dời từ `forms`), `autoxide_kernel::slug` (dời từ `content`) | 41 (commit chuẩn bị) | 42–49 |
| `tenancy::contact_key` (HMAC số điện thoại / email theo khách) | 45 | 47 |
| `Resource::image`, node `embed` của `content` | 51 (sau 33) | 41 (`catalog.product`) |
| `navigation` (`Module::navigation`), `work::WorkSource`, `finder::FinderSource`, `summary::SummaryMetric`, `EventInfo::activity` | 51 (sau 34–35) | 41–49 |
| `x-autoxide-filters`, `kernel_saved_views`, `BulkAction`, `resolve`, `kernel_work_states`, `SetupStep`, `/me/can`, `ApiKeyKind`, `ActorKind`, `kernel_approval_requests`, `kernel_record_notes` | 51 (D-201) | 41–49, 52 |
| `custom_fields` (`Module::custom_field_targets`, `CustomFields::validate`, `index_migration`, `CustomIndex`), `tags` (`taggable`, `kernel_tags`), `Module::overridable_texts` + `kernel_text_overrides` (D-235) | 51 (P19–P21) | 37 (câu banner), 41–49, 53, 59–62, 67 (loại nội dung) |
| `blocks` (`BlockType`, `Module::blocks`, `BlockData`, `GET /kernel/blocks`) | 67 (D-229) | 41, 43, 45 (khai khối) |
| `voice::VoiceProvider` + bộ giả (D-232) | 61 | — |
| `signature::SignatureProvider` + bộ giả (D-232) | 59 (luồng ký trong `compliance`) | 46, 49 |
| `entitlement::AddonBilling` (D-232) | 65 | 35 (`plans` cộng dòng phí thêm) |

## Việc chỉ chủ dự án làm được

Đặc tả 30 có bảng đầy đủ. Agent không tự làm:

- đổi repo sang công khai; bật Private vulnerability reporting, secret scanning, push protection;
- đưa địa chỉ liên hệ cho Code of Conduct;
- lịch sử git: **đã quyết giữ nguyên** (chủ dự án, 13/11/2026, D-198) — không viết lại, không hỏi lại;
- cấp tài khoản thử của đối tác (cổng thanh toán, hãng vận chuyển, hoá đơn điện tử, SMS, eKYC, sàn) khi
  tới phần bộ nối.
