# Bước 37 — Tuân thủ pháp luật Việt Nam (`compliance`) và thông báo (`notifications`)

> Lộ trình: `docs/ROADMAP.md` bước 37 · Thiết kế: `docs/research/business-foundation.md` trụ 5, mục 4
> (Bộ Công Thương), mục 7 (D-190), mục 9 (D-194) · Quyết định: D-185, D-186, D-187, D-190, D-193, D-194
> (và D-046, D-054, D-109, D-112, D-123 của phần đã có)
> Nguồn đối chiếu thiết kế 2: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới
> `docs/research/ui-trace/`.
> Đã gộp các quyết định: D-202, D-206, D-217, D-220; đối chiếu thiết kế 2 (D-208…D-218) (D-228).

File này có hai phần: **Phần A — module `compliance`**, **Phần B — module `notifications`**, cộng
**Phần C — sửa module và kernel có sẵn**. Ba phần dùng chung một danh sách commit, test, tiêu chí xong.

## Mục tiêu

Xong bước này thì một khách (cửa hàng, chuỗi, khách SaaS, sàn) chạy được ở Việt Nam mà không vi
phạm các nghĩa vụ nền: người bán / người livestream / người tiếp thị liên kết phải qua KYC mới được
bán / phát; mỗi tên miền có hồ sơ pháp lý, danh sách kiểm và bộ hồ sơ để thông báo (logo xanh) hay đăng
ký (logo đỏ) với Bộ Công Thương, logo hiện ở chân trang qua endpoint công khai; có trang bắt buộc
từ mẫu; điều khoản, chính sách, quy chế, hợp đồng có phiên bản và bằng chứng ai chấp nhận bản nào;
báo vi phạm SHTT / hàng giả và khiếu nại người tiêu dùng có hạn xử lý và lưu vết; tài khoản đăng
nội dung xác thực bằng SMS khi khách bật; sổ mục đích / đồng ý, thời hạn lưu, quy trình sự cố 72 giờ;
nội dung có AI gợi ý mang `ai_assisted`; một module thông báo nhiều kênh tách tin giao dịch và tin
quảng cáo, dùng chung sổ đồng ý với `newsletter`, có kênh đẩy cho ứng dụng di động (D-220); mẫu mail và
thông báo do khách tự soạn qua một bộ dựng mẫu an toàn dùng chung trong kernel (D-206); yêu cầu của chủ thể
dữ liệu qua kênh khác có sổ và hạn (D-217). Mọi điểm luật hay đổi là **dữ liệu có ngày hiệu
lực**, không viết cứng trong code (D-186).

Phục vụ cả năm mô hình: khách độc lập (website bán hàng → thông báo), chuỗi (mỗi tên miền một hồ sơ),
SaaS (mỗi khách tự lo hồ sơ của họ), sàn (đăng ký, KYC người bán, quy chế, khiếu nại).

## Bắt đầu khi

- Bước 34 xong (bảng thứ tự D-192). Cần từ 33: `TenantId`, `TenantContext`, công cụ lọc theo khách,
  bảng `kernel_tenants` / `kernel_tenant_domains` / `kernel_placements`, mã hoá AEAD có phiên bản khoá
  của `kernel::secret`, kho secret theo khách. Cần từ 34: `OrgId`, `Principal` mang tenant + `Scope`,
  `dyn OrgDirectory` (loại nút `seller`…), kiểu `Address` và `dyn AdministrativeUnits` của
  `autoxide_address::api`, danh tính hai lớp (nhân sự / người mua).
- Làm song song với 35 và 36. Mục cấp phát của 35 chờ `KycGate` của bước này; bước 45 chờ
  `dyn Notifier`; 46 và 49 chờ `KycGate`, `AcceptanceGate`, case.
- Không cần bộ nối đối tác thật (eKYC, SMS brandname, Zalo): bước này chỉ dựng trait + bản giả
  (D-191: bộ nối là module riêng, làm khi chủ dự án cấp tài khoản thử).

## Phạm vi

**Mới:**

- Kernel: `crates/kernel/src/identity.rs` (trait `IdentityVerifier`), `crates/kernel/src/sms.rs`
  (trait `SmsSender`, kiểu `PhoneNumber`), `StartContext::optional`, `Module::retention`, cột cư trú dữ
  liệu trên bảng mặt điều khiển của bước 33; bộ dựng mẫu `autoxide_kernel::template` (D-206, mục 2).
- Testkit: `FakeIdentityVerifier`, `FakeSmsSender` (`crates/testkit/src/identity.rs`, `sms.rs`); bản giả
  `MobilePushSender` cho test (D-220).
- Crate `modules/compliance` (`autoxide-compliance`), crate `modules/notifications`
  (`autoxide-notifications`); bảng `compliance_…`, `notifications_…`.
- Tài liệu cho người ngoài: `docs/COMPLIANCE.md` + `docs/COMPLIANCE.vi.md` (gồm sổ tay sự cố 72 giờ),
  `docs/NOTIFICATIONS.md` + `docs/NOTIFICATIONS.vi.md`.

**Sửa:** `content` (mẫu trang bắt buộc, `pub mod api` cho ảnh chụp trang đã đăng, `ai_assisted`, ẩn
theo vụ việc), `media` (`alt_ai_assisted`), `comments` (xác thực khi đăng qua `PostingVerifier` của kernel
mà `compliance` đăng ký — D-202; ẩn theo vụ việc), `newsletter` (sổ đồng ý chung, chiến dịch dùng bộ dựng
mẫu — D-206), mọi module giữ dữ liệu cá nhân (khai `Module::retention`), `crates/app/src/modules.rs` (thứ
tự đăng ký), `docs/MODULE_GUIDE.md` + `.vi.md`, `docs/COMMENTS.md` + `.vi.md` (D-202), `CHANGELOG.md`,
`STACK.md` (thư viện web push nếu thêm; thư viện mẫu và `mrml` — D-206), `docs/openapi.json`,
`.env.example`.

**Ngoài phạm vi:** bộ nối VNeID / eKYC / SMS brandname / Zalo thật; nộp hồ sơ lên cổng Bộ Công Thương
(không có API công khai — con người nộp); chiến dịch SMS / ZNS hàng loạt (chỉ có gửi từng tin; email
hàng loạt vẫn là `newsletter`); nhận từ khoá "TC" trả lời SMS (cần bộ nối có tin đến); kiểm danh sách
"không quảng cáo" quốc gia (ghi ở "Agent tự quyết"); SSE / realtime cho hộp thư trong ứng dụng (bước
40); thuế khấu trừ, chứng từ (36); đánh giá sau mua (45 dùng `ModerationPolicy` của bước này); thông
tin người bán trên trang sản phẩm (46 khai "fact" cho danh sách kiểm); module `email` riêng — gửi mail vẫn
là `mail` (`dyn Mailer`), nơi khách sửa mẫu vẫn là `notifications_templates`, chiến dịch vẫn là `newsletter`
(D-206); bộ cài đặt `MobilePushSender` cho Firebase Cloud Messaging / Apple Push Notification service (module
riêng, cần khoá của nhà cung cấp — khi đó dừng lại hỏi chủ dự án, D-220).

## Thiết kế

### 0. Thứ tự đăng ký và hướng phụ thuộc

```
mail, ai, files, auth, audit, idempotency, webhooks, media, content, forms,
organizations, address, plans, tax, ledger, invoices,
[connector eKYC / SMS / ZNS nếu có — công bố dyn IdentityVerifier / dyn SmsSender / dyn ZnsSender]
compliance      — require: dyn Mailer, dyn OrgDirectory, dyn AdministrativeUnits, dyn PublishedEntries (content)
                  optional: dyn IdentityVerifier, dyn SmsSender
                  provide:  dyn KycGate, dyn PostingGate, dyn AcceptanceGate, dyn ConsentLedger,
                            dyn AccountVerification, dyn ModerationPolicy, dyn ComplianceRules, dyn ComplianceFacts
                  đăng ký:  PostingVerifier của kernel (context.posting_verifier, D-202)
notifications   — require: dyn Mailer, dyn Users (auth), dyn ConsentLedger, dyn AccountVerification, dyn ComplianceRules
                  optional: dyn SmsSender, dyn ZnsSender, dyn MobilePushSender (D-220);  provide: dyn Notifier
newsletter      — DỜI xuống sau notifications (cần dyn ConsentLedger)
catalog, inventory, … (như bảng tên dùng chung)
```

`comments` không `require::<dyn PostingGate>` nên không phải dời thứ tự đăng ký: nó hỏi `PostingVerifier`
của kernel, và chỗ giữ `PostingVerification` thấy bộ hỏi cả khi `compliance` khởi động sau (D-202, xem C2).

Vì sao: `notifications` cần sổ đồng ý của `compliance` → `compliance` đứng trước. `compliance` vì vậy
**không** dùng `dyn Notifier`: mail của nó (kết quả KYC, vụ việc, nhắc hạn sự cố) gửi thẳng bằng
`dyn Mailer` + `MailTemplate` như `forms` / `comments` đang làm; OTP gửi thẳng bằng `dyn SmsSender`.
Việc ghi chéo giữa module đi qua event (luật kiến trúc 3); gọi đồng bộ chỉ khi cần trả lời ngay (cổng
chặn, kiểm đồng ý lúc gửi). `compliance` và `notifications` là module nền: bước 35 không được cho tắt
chúng theo gói (ghi vào "Đề xuất" cho agent chính).

### 1. Kernel — kiểu và trait mới (phác thảo, tra API thật bằng skill `check-library-api`)

```rust
// crates/kernel/src/identity.rs — như `ai.rs`: trait trung lập, nhà cung cấp ở module khác.
pub enum PartyType { Individual, HouseholdBusiness, Organization }
pub enum VerificationMethod { Vneid, Ekyc, BusinessRegistry, Manual }
pub struct VerificationRequest { pub tenant: TenantId, pub attempt_id: Uuid, pub party: PartyType,
    pub method: VerificationMethod, pub return_url: Option<String>, pub locale: String }
pub enum VerificationSession { Redirect { url: String, expires_at: DateTime<Utc> },
    Sdk { token: String, expires_at: DateTime<Utc> }, Pending }
pub enum OutcomeStatus { Verified, Rejected { reason_code: String }, Pending }
pub struct VerificationOutcome { pub provider_reference: String, pub status: OutcomeStatus,
    pub verified_name: Option<String>, pub document_kind: Option<String>,
    pub document_number: Option<String>,      // compliance chỉ lưu băm + 4 số cuối, không lưu nguyên
    pub assurance: Option<String>,            // vd "vneid_level_2"
    pub expires_at: Option<DateTime<Utc>> }
#[async_trait::async_trait]
pub trait IdentityVerifier: Send + Sync {
    fn provider(&self) -> &'static str;
    fn methods(&self) -> Vec<VerificationMethod>;
    async fn start(&self, request: VerificationRequest) -> Result<VerificationSession, Error>;
    async fn fetch(&self, provider_reference: &str) -> Result<VerificationOutcome, Error>;
    /// Kiểm chữ ký callback của nhà cung cấp; sai chữ ký → `identity_callback_invalid` (401).
    async fn verify_callback(&self, headers: &http::HeaderMap, body: &[u8]) -> Result<VerificationOutcome, Error>;
}

// crates/kernel/src/sms.rs
pub struct PhoneNumber(String);               // E.164, vd "+84912345678"
impl PhoneNumber {
    pub fn parse(raw: &str, default_country_code: &str) -> Option<Self>; // "0912 345 678" → "+84912345678"
    pub fn calling_code(&self) -> &str;       // "84"
    pub fn masked(&self) -> String;           // "+8491****678" — dạng duy nhất được ghi log
}
pub enum SmsPurpose { Otp, Transactional, Marketing }
pub struct SmsMessage { pub tenant: TenantId, pub to: PhoneNumber, pub text: String,
    pub purpose: SmsPurpose, pub reference: String /* chống gửi trùng ở nhà cung cấp */ }
pub struct SmsReceipt { pub provider_reference: String }
#[async_trait::async_trait]
pub trait SmsSender: Send + Sync {
    fn provider(&self) -> &'static str;
    /// Lỗi `ErrorKind::Unavailable` = tạm thời (thử lại); `Validation` = vĩnh viễn (số sai, bị chặn).
    async fn send(&self, message: SmsMessage) -> Result<SmsReceipt, Error>;
}
```

API mới cần thêm ở `module.rs`:

- `StartContext::optional::<T>() -> Option<Arc<T>>` — như `require` nhưng không lỗi khi chưa ai công
  bố (cho nhà cung cấp tuỳ chọn: eKYC, SMS, ZNS). Không có thì module dùng bản "đã tắt" tự viết: mọi
  lời gọi trả `identity_verification_disabled` / `sms_disabled` (503).
- `Module::retention(&self) -> Vec<RetentionPolicy>` mặc định rỗng (D-038), gọi sau `configure`:
  `RetentionPolicy { name: "<module>.<loại>", description, category: DataCategory /* Basic | Sensitive
  | Security | Accounting */, keep: Keep /* Days(u32) | UntilDeleted | Forever */, setting:
  Option<&'static str> }`. Kernel gom thành `RetentionCatalog`, `compliance` đọc qua
  `StartContext::retention_catalog()` (sổ có cả phần module đăng ký sau, như `PersonalDataExport`).

Migration kernel mới (không sửa migration của bước 33): `kernel_placements.country_code` (TEXT NULL,
ISO 3166-1 alpha-2), `kernel_tenants.data_residency` (TEXT NULL; `VN` = phải lưu tại Việt Nam). Chọn
nơi đặt khi cấp phát (35) và `autoxide copy` / dời khách (33) từ chối nơi có `country_code` khác
`data_residency` hoặc không rõ → lỗi `tenant_residency_violation` (409). Có `down`.

Mã lỗi kernel mới (bản dịch ở `crates/kernel/locales/vi.json`): `identity_verification_disabled`
("Chưa cấu hình nhà cung cấp xác thực danh tính."), `identity_callback_invalid` ("Chữ ký callback của
nhà cung cấp không hợp lệ."), `sms_disabled` ("Máy chủ chưa cấu hình gửi SMS."),
`tenant_residency_violation` ("Khách yêu cầu lưu dữ liệu tại {country}; nơi lưu này không đáp ứng.").

### 2. Kernel — bộ dựng mẫu `autoxide_kernel::template` (D-206)

Không có module `email` riêng (Phạm vi). Bước này thêm **một bộ dựng mẫu dùng chung** để mẫu do khách tự
soạn có biến, điều kiện, vòng lặp, và mail HTML hiển thị đúng ở mọi trình đọc. `notifications` (Phần B) và
`newsletter` (C3) dùng nó.

- **Ngôn ngữ mẫu an toàn cho người dùng tự viết** (mẫu là dữ liệu không tin cậy):
  - biến `{{ customer.name }}`, điều kiện `{% if %}`, vòng lặp `{% for line in order.lines %}`, bộ lọc
    định dạng của kernel (`money`, `date`, `number` theo ngôn ngữ và múi giờ của nút);
  - **không** đọc tệp, gọi mạng, gọi hàm tuỳ ý, nạp mẫu khác ngoài danh sách khối dựng sẵn;
  - giới hạn: độ dài mẫu (`AUTOXIDE_TEMPLATE_MAX_BYTES`, mặc định 64 KiB), số bước chạy, độ sâu vòng lặp,
    độ dài kết quả — vượt thì lỗi `template.limit_exceeded`, không treo worker;
  - mặc định **thoát HTML** mọi giá trị chèn vào bản HTML; muốn chèn HTML phải là khối dựng sẵn của hệ
    thống (vd bảng dòng hàng), không có bộ lọc `raw` cho dữ liệu người dùng.
- **Thư viện:** cần một thư viện mẫu (vài chục dòng tự viết không đủ cho điều kiện, vòng lặp, giới hạn
  chạy); chọn giữa `liquid` và `minijinja` — "Agent tự quyết" #9. Chỉ một ngôn ngữ mẫu trong cả dự án.
- **Bố cục HTML:** khung mail chung (logo, màu chủ đạo, chân thư có địa chỉ người gửi theo hồ sơ pháp lý
  của nút) viết bằng MJML, dựng bằng `mrml` (MJML viết bằng Rust) để chạy đúng trên Outlook, Gmail, điện
  thoại. Khách chỉnh logo / màu / chân thư bằng dữ liệu, không sửa MJML. **Luôn kèm bản chữ thuần** sinh
  từ cùng mẫu.
- **Hợp đồng biến:** loại thông báo khai **schema biến** (JSON Schema, cùng chỗ với `NotificationKind`) và
  một bộ dữ liệu mẫu để xem trước. Lưu mẫu thì kiểm: cú pháp đúng, chỉ dùng biến có trong schema, biến bắt
  buộc của loại đó có mặt (vd link đặt lại mật khẩu) — sai thì gom lỗi một lần (D-083) với vị trí dòng,
  cột: `template.syntax_error`, `template.unknown_variable`, `template.required_variable_missing`.
- Kernel chỉ dựng chữ, không biết khái niệm đơn hàng hay sản phẩm: dữ liệu do module khai báo loại đưa vào.

---

## Phần A — module `compliance`

### A1. Hợp đồng `autoxide_compliance::api` (phác thảo)

```rust
pub enum Activity { Sell, Livestream, Affiliate }
pub enum KycSubject { User(Uuid), Org(OrgId) }
#[async_trait] pub trait KycGate: Send + Sync {
    /// Ok khi luật đang hiệu lực không đòi KYC cho `activity`, hoặc subject có hồ sơ `verified` còn hạn.
    /// Ngược lại `compliance.kyc_required` (403, kèm `activity`, `profile_status`).
    async fn require_verified(&self, tenant: TenantId, subject: KycSubject, activity: Activity) -> Result<(), Error>;
    async fn status(&self, tenant: TenantId, subject: KycSubject) -> Result<KycStatus, Error>;
}
pub enum VerificationLevel { None, Phone, Identity }   // Identity ⊃ Phone
pub enum PostingSurface { Comments, Reviews, Livestream }
#[async_trait] pub trait PostingGate: Send + Sync {
    /// `None` principal = khách vãng lai. Thiếu mức khách đặt → `compliance.verification_required` (403).
    async fn check(&self, tenant: TenantId, principal: Option<&Principal>, surface: PostingSurface) -> Result<(), Error>;
}
#[async_trait] pub trait AccountVerification: Send + Sync {
    async fn level(&self, tenant: TenantId, user_id: Uuid) -> Result<VerificationLevel, Error>;
    /// Số đã xác thực, cho `notifications` gửi SMS / ZNS. Không có → None.
    async fn verified_phone(&self, tenant: TenantId, user_id: Uuid) -> Result<Option<PhoneNumber>, Error>;
}
pub enum DocumentKind { TermsOfUse, PrivacyPolicy, GeneralConditions, PlatformRules, SellerAgreement }
#[async_trait] pub trait AcceptanceGate: Send + Sync {
    /// Subject đã chấp nhận bản hiện hành cần chấp nhận của `kind`; không → `compliance.acceptance_required` (403).
    async fn require_accepted(&self, tenant: TenantId, subject: KycSubject, kind: DocumentKind) -> Result<(), Error>;
}
pub enum ConsentSubject { User(Uuid), Email(String), Phone(PhoneNumber) }
pub struct ConsentKey { pub purpose: String /* "marketing" */, pub scope: String /* "" | "newsletter.list:<id>" */,
    pub channel: String /* "" | email | sms | zns | push | in_app */ }
pub struct ConsentEvidence { pub text: String, pub text_version: String, pub source: String,
    pub ip_fingerprint: Option<String> }
#[async_trait] pub trait ConsentLedger: Send + Sync {
    /// Idempotent theo `source_ref`: gọi lại cùng mã → không ghi thêm, trả trạng thái hiện tại.
    async fn grant(&self, tenant: TenantId, subject: ConsentSubject, key: ConsentKey, evidence: ConsentEvidence, source_ref: &str) -> Result<Consent, Error>;
    async fn withdraw(&self, tenant: TenantId, subject: ConsentSubject, key: ConsentKey, source: &str, source_ref: &str) -> Result<Consent, Error>;
    /// Có đồng ý còn hiệu lực ở đúng khoá hoặc khoá rộng hơn (scope ""/channel "" bao các con) và
    /// KHÔNG có rút lại ở khoá rộng hơn (rút ở `marketing` + channel "" chặn mọi kênh).
    async fn is_granted(&self, tenant: TenantId, subject: &ConsentSubject, key: &ConsentKey) -> Result<bool, Error>;
    async fn granted_many(&self, tenant: TenantId, subjects: &[ConsentSubject], key: &ConsentKey) -> Result<Vec<bool>, Error>;
    async fn get(&self, tenant: TenantId, consent_id: Uuid) -> Result<Option<Consent>, Error>;
}
#[async_trait] pub trait ModerationPolicy: Send + Sync {
    /// Lý do ẩn đánh giá / nội dung người tiêu dùng có thuộc danh sách được phép (dữ liệu) không;
    /// không → `compliance.hide_reason_not_allowed` (422). Bước 45 dùng cho đánh giá.
    async fn check_hide_reason(&self, tenant: TenantId, surface: &str, reason: &str) -> Result<(), Error>;
}
#[async_trait] pub trait ComplianceRules: Send + Sync {
    /// Bản quy tắc hiệu lực lúc `at` cho vùng pháp lý của khách (mặc định "vn"); None = không có.
    async fn effective(&self, tenant: TenantId, key: &str, at: DateTime<Utc>) -> Result<Option<Rule>, Error>;
}
/// "Fact" cho danh sách kiểm, module sau đăng ký (vd 46: "marketplace.seller_info_on_product").
pub trait ComplianceFacts: Send + Sync { fn register(&self, key: &'static str, fact: Arc<dyn Fact>) -> Result<(), Error>; }
#[async_trait] pub trait Fact: Send + Sync { async fn holds(&self, tenant: TenantId, website_id: Uuid) -> Result<bool, Error>; }
```

### A2. Bảng dữ liệu

Quy ước chung: id UUIDv7 sinh trong ứng dụng; thời gian UTC (`timestamp_with_time_zone` như mọi
module); JSON lưu `TEXT` (như `newsletter_subscriptions.consent`); "enum" lưu `TEXT`, service kiểm;
mọi unique / index bắt đầu bằng `tenant_id`; không foreign key xuyên module (chỉ giữ id, chuỗi đối
tượng `<module>.<loại>:<id>` của D-123). Cột tham gia unique **không để NULL** — dùng `''` (NULL khác
nhau trong unique trên cả hai database). Ba migration: `m0001_kyc_and_verification`,
`m0002_legal_and_documents`, `m0003_privacy_and_cases`, mỗi cái có `down`; thêm `outbox_migration`,
`jobs_migration`, `throttle_migration("compliance_throttle", "compliance")`.

**Dữ liệu luật dùng chung mọi khách (mặt điều khiển, không `tenant_id`):**

| Bảng | Cột chính | Ghi chú |
|---|---|---|
| `compliance_rules` | `id`, `jurisdiction` (`vn`), `key`, `version` INT, `effective_from`, `effective_to` NULL, `body` JSON, `legal_basis` TEXT, `needs_legal_review` BOOL, `source` (`bundled` / `import`), `imported_at` | unique (`jurisdiction`,`key`,`version`); index (`jurisdiction`,`key`,`effective_from`). Chỉ thêm, không sửa bản đã nhập |

Khoá quy tắc (gói `vn` đóng trong crate ở `modules/compliance/rules/vn/*.json`, nạp bằng task / job):
`kyc.required_activities` (`{"sell":true,"livestream":true,"affiliate":true}`, hiệu lực 2026-07-01),
`moit.checklist.sales` và `moit.checklist.platform` (mục kiểm, xem A4), `posting.phone_countries`
(`["84"]`, tiền tố di động hợp lệ), `cases.deadlines` (giờ / ngày làm việc theo `kind`), `privacy.incident`
(`{"report_within_hours":72,"remind_at_hours":[24,48,60,68]}`), `privacy.default_purposes`,
`privacy.retention_limits`, `privacy.consent_proof_days`, `moderation.hide_reasons`,
`notifications.send_windows` (Phần B), `notifications.marketing_labels`.

**Cấp khách hàng (`tenant_id` NOT NULL):**

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `compliance_policies` | `tenant_id`, `key` (`jurisdiction`, `posting.comments`, `posting.reviews`, `posting.livestream`, `posting.max_accounts_per_phone`, `cases.auto_restrict`, `complaints.contact`, `privacy.contact`, `incident.alert_emails`), `value` JSON, `updated_by`, `updated_at` | PK (`tenant_id`,`key`) |
| `compliance_kyc_profiles` | `id`, `tenant_id`, `subject_kind` (`user`/`org`), `subject_id`, `party_type`, `status` (`draft`/`pending`/`needs_info`/`verified`/`rejected`/`expired`/`revoked`; `needs_info` — A-c9), `method`, `provider`, `provider_reference`, `verified_name`, `document_kind`, `document_hash` (SHA-256 của số đã chuẩn hoá), `document_last4`, `assurance`, `verified_at`, `expires_at`, `reason_code`, `reviewed_by`, `created_at`, `updated_at` | unique (`tenant_id`,`subject_kind`,`subject_id`); index (`tenant_id`,`status`,`id`), (`tenant_id`,`document_hash`) |
| `compliance_kyc_attempts` | `id`, `tenant_id`, `profile_id`, `method`, `provider`, `provider_reference`, `status`, `result_code`, `result` JSON (chỉ trường trong allowlist), `created_at`, `completed_at` | index (`tenant_id`,`profile_id`,`id`); unique (`tenant_id`,`provider`,`provider_reference`) khi có — dùng `''` khi chưa có và loại `''` khỏi kiểm ở service |
| `compliance_kyc_callbacks` | `id`, `tenant_id`, `provider`, `received_at`, `body` (nguyên văn, đã kiểm chữ ký), `processed_at`, `attempt_id` | index (`tenant_id`,`received_at`); xoá theo `privacy.retention_limits` |
| `compliance_kyc_documents` | `id`, `tenant_id`, `profile_id`, `storage_key` (`compliance/kyc/<tenant>/<id>`), `media_type`, `size`, `delete_after`, `deleted_at`, `created_at` | chỉ cho `method = manual`; job xoá file sau quyết định + `AUTOXIDE_COMPLIANCE_KYC_DOCUMENT_RETENTION_DAYS` |
| `compliance_account_verifications` | `tenant_id`, `user_id`, `phone` (E.164) NULL, `phone_key` (SHA-256 của số, `''` khi chưa có), `phone_verified_at`, `identity_verified_at`, `identity_profile_id`, `updated_at` | PK (`tenant_id`,`user_id`); index (`tenant_id`,`phone_key`) |
| `compliance_phone_challenges` | `id`, `tenant_id`, `user_id`, `phone`, `code_hash` (SHA-256 + pepper từ `SecretKey::derive("compliance.otp")`), `attempts`, `expires_at`, `consumed_at`, `created_at` | index (`tenant_id`,`user_id`,`created_at`) |
| `compliance_legal_profiles` | `id`, `tenant_id`, `org_id` NULL (NULL = cấp khách), `org_key` (`org_id` hoặc `''`), `entity_type`, `legal_name`, `trade_name`, `tax_code`, `registration_number`, `registration_issued_on`, `registration_issued_by`, `address` JSON (`Address` của `autoxide_address::api`, kèm tên tại lúc ghi), `representative_name`, `representative_title`, `phone`, `email`, `kyc_profile_id` NULL, `created_at`, `updated_at` | unique (`tenant_id`,`org_key`) |
| `compliance_websites` | `id`, `tenant_id`, `org_key`, `domain` (chữ thường, IDN dạng punycode), `domain_id` NULL (của `kernel_tenant_domains`), `channel` (`website`/`mobile_app`), `website_kind` (`sales` → thông báo / logo xanh; `platform` → đăng ký / logo đỏ), `profile_id`, `moit_status` (`not_submitted`/`submitted`/`needs_supplement`/`notified`/`registered`/`rejected`/`revoked`), `moit_reference`, `submitted_at`, `decided_at`, `logo_link_url`, `logo_image_url`, `logo_snippet` (đã làm sạch), `checklist_version`, `created_at`, `updated_at` | unique (`tenant_id`,`domain`) |
| `compliance_website_pages` | `tenant_id`, `website_id`, `page_kind`, `resource` (`content.entry:<id>`), `linked_at` | PK (`tenant_id`,`website_id`,`page_kind`) |
| `compliance_documents` | `id`, `tenant_id`, `org_key`, `kind` (5 loại `DocumentKind`), `audience` (`user`/`buyer`/`seller`), `title`, `current_version` INT NULL, `created_at`, `updated_at` | unique (`tenant_id`,`org_key`,`kind`) |
| `compliance_document_versions` | `id`, `tenant_id`, `document_id`, `version` INT, `locale`, `source` (`content.entry:<id>`), `source_revision_id`, `title`, `body` JSON (ảnh chụp thân bài), `plain_text`, `sha256`, `effective_from`, `requires_reacceptance` BOOL, `published_by`, `published_at` | unique (`tenant_id`,`document_id`,`version`,`locale`); không bao giờ UPDATE / DELETE (trừ xoá khách theo luật lưu trữ) |
| `compliance_acceptances` | `id`, `tenant_id`, `document_id`, `version`, `version_id`, `subject_kind`, `subject_id`, `accepted_by` (user), `channel` (`web`/`mobile_app`/`api`/`mcp`/`in_store`), `method` (`checkbox`/`click_wrap`/`signature`), `ip_fingerprint`, `user_agent` (≤ 300 ký tự), `sha256` (của bản đã chấp nhận), `accepted_at` | unique (`tenant_id`,`version_id`,`subject_kind`,`subject_id`); index (`tenant_id`,`subject_kind`,`subject_id`,`document_id`) |
| `compliance_purposes` | `id`, `tenant_id`, `key`, `category` (`service`/`marketing`/`analytics`/`legal`/`kyc`), `legal_basis` (`consent`/`contract`/`legal_obligation`/`vital_interest`/`public_interest`), `data_categories` JSON, `channels` JSON, `description` JSON (`{"en":…, "vi":…}`), `retention` TEXT, `active`, `updated_at` | unique (`tenant_id`,`key`). Chỉ giữ phần khách thêm / ghi đè; danh sách hiệu lực = `privacy.default_purposes` ∪ bảng này |
| `compliance_consents` | `id`, `tenant_id`, `subject_kind` (`user`/`email`/`phone`), `subject_key` (user id, hoặc SHA-256 của email / số đã chuẩn hoá), `subject_value` NULL (email / số; NULL sau khi xoá theo yêu cầu), `user_id` NULL, `purpose`, `scope`, `channel`, `state` (`granted`/`withdrawn`), `granted_at`, `withdrawn_at`, `updated_at` | unique (`tenant_id`,`subject_kind`,`subject_key`,`purpose`,`scope`,`channel`) |
| `compliance_consent_records` | `id`, `tenant_id`, `consent_id`, `action` (`grant`/`withdraw`), `evidence` JSON, `source`, `source_ref`, `actor` JSON, `recorded_at` | unique (`tenant_id`,`source_ref`,`action`); chỉ thêm |
| `compliance_cases` | `id`, `tenant_id`, `kind` (`ip_infringement`/`counterfeit`/`prohibited_goods`/`consumer_complaint`/`other`), `target` (`<module>.<loại>:<id>` hay `''`), `respondent_org_key`, `reporter_kind` (`guest`/`user`/`rights_holder`/`authority`), `reporter_user_id`, `reporter_name`, `reporter_email`, `reporter_phone`, `claim` TEXT, `rights` JSON (văn bằng, link), `status` (`received`/`under_review`/`awaiting_response`/`decided`/`closed`/`withdrawn`), `restricted` BOOL, `decision` (`upheld`/`rejected`/`partially_upheld`/`''`), `ack_due_at`, `respond_by`, `decide_by`, `decided_at`, `decided_by`, `tracking_hash` (băm token tra cứu của người báo), `created_at`, `updated_at` | index (`tenant_id`,`status`,`decide_by`), (`tenant_id`,`target`), (`tenant_id`,`respondent_org_key`,`id`) |
| `compliance_case_entries` | `id`, `tenant_id`, `case_id`, `kind` (`note`/`status`/`response`/`evidence`/`restriction`/`decision`/`mail`), `author_kind`, `author_user_id`, `body`, `files` JSON (id của `files`), `created_at` | index (`tenant_id`,`case_id`,`id`); chỉ thêm |
| `compliance_disclosures` | `id`, `tenant_id`, `authority`, `request_reference`, `legal_basis`, `requested_at`, `subject` (`org:<id>`/`user:<id>`), `fields` JSON (TÊN trường đã cung cấp, không giá trị), `disclosed_by`, `disclosed_at` | index (`tenant_id`,`disclosed_at`) |
| `compliance_incidents` | `id`, `tenant_id` (sự cố toàn nền tảng ghi một dòng cho từng khách bị ảnh hưởng, vì mỗi khách là bên kiểm soát dữ liệu của mình), `title`, `description`, `severity`, `status` (`open`/`contained`/`reported`/`closed`), `detected_at`, `report_due_at` (= `detected_at` + giờ của `privacy.incident`), `authority_notified_at`, `authority_reference`, `subjects_notified_at`, `data_categories` JSON, `affected_count`, `created_by`, `updated_at` | index (`tenant_id`,`status`,`report_due_at`) |
| `compliance_incident_entries` | `id`, `tenant_id`, `incident_id`, `body`, `author_user_id`, `created_at` | chỉ thêm |
| `compliance_subject_requests` (D-217) | `id`, `tenant_id`, `kind` (`access`/`erasure`/`rectification`/`objection`/`restriction`), `channel` (`email`/`post`/`phone`/`authority`/`other`), `subject` (email / số / user_id), `received_at`, `due_at`, `status` (`received`/`in_progress`/`fulfilled`/`rejected`), `reason`, `job_id`, `handled_by` | chỉ cho yêu cầu qua kênh khác — xem A3 "Dữ liệu cá nhân" |

Cùng `compliance_events`, `compliance_jobs`, `compliance_throttle`.

### A3. Luồng chính

**KYC.** `POST …/kyc/profiles` (hay `/me/kyc`) tạo hồ sơ `draft` → `POST …/sessions` gọi
`IdentityVerifier::start` NGOÀI transaction, ghi attempt `pending`, trả `VerificationSession` (URL VNeID /
token SDK eKYC) → kết quả về bằng callback (`POST /api/v1/compliance/kyc/callbacks/{provider}`, công
khai, kiểm chữ ký, ghi nguyên văn vào `compliance_kyc_callbacks`, xử lý qua job
`compliance.kyc_process`) hoặc job `compliance.kyc_poll` (`fetch`, backoff). `Verified` → hồ sơ
`verified`, `expires_at` = của nhà cung cấp hoặc `verified_at` + hạn trong `kyc.required_activities`;
nếu subject là user thì cập nhật `identity_verified_at` của `compliance_account_verifications`. Không có
nhà cung cấp → chỉ `method = manual`: người dùng tải ảnh giấy tờ (`files`), người có
`compliance.kyc.review` duyệt bằng **phiên đăng nhập** (không nhận API key, như `ai_reviewed` của
`content`), ảnh xoá sau `delete_after`. Lưu tối thiểu: không lưu ảnh khuôn mặt / dữ liệu sinh trắc học
(dữ liệu nhạy cảm, Luật 91/2025) — chỉ kết quả, mã tham chiếu, tên đã xác thực, băm + 4 số cuối giấy tờ.
Cùng `document_hash` đã `verified` ở hồ sơ khác → giữ `pending` chờ duyệt tay (`reason_code =
duplicate_document`). Job hằng ngày `compliance.kyc_expiry` → `expired` + event + mail. **Cần bổ sung**
(A-c9): `POST …/kyc/profiles/{id}/request-info {fields, message}` đưa hồ sơ về `needs_info`; người nộp nhận
thông báo, nộp bổ sung thì hồ sơ về lại `pending`.

**Cổng chặn.** `KycGate::require_verified` đọc `kyc.required_activities` hiệu lực lúc gọi (trước
01/7/2026 không chặn — ngày nằm trong dữ liệu). Bước 37 tự dùng cổng ở: `PostingGate` với
`Livestream`; 41/46/49 gọi khi đăng bán, mở gian hàng, phát, tạo link tiếp thị.

**Xác thực số điện thoại (NĐ 147/2024).** `POST /me/phone-verifications {phone}` → chuẩn hoá bằng
`PhoneNumber::parse(raw, "84")`, kiểm `posting.phone_countries` và tiền tố di động → throttle (số / giờ,
user / ngày, IP / giờ) → sinh mã 6 số, lưu băm, **gửi SMS đồng bộ ngoài transaction** (`purpose = Otp`,
timeout 10 s; lỗi → xoá challenge, trả `sms_disabled` hay lỗi tạm thời 503) — không đưa mã vào job (bảng
job đọc được qua API quản trị và dead-letter queue). `POST /me/phone-verifications/{id}/confirm {code}`
so sánh hằng thời gian, tối đa `OTP_MAX_ATTEMPTS`, dùng một lần (`UPDATE … WHERE consumed_at IS NULL`
qua `rows`), rồi ghi `phone_verified_at`. `posting.max_accounts_per_phone` (mặc định 1) chặn một số xác
thực cho nhiều tài khoản trong cùng khách (`compliance.phone_in_use`).

**Cổng đăng nội dung.** `PostingGate::check` đọc `posting.<surface>` (`none`/`phone`/`identity`, mặc
định `none`) → so với `AccountVerification::level`; khách vãng lai chỉ qua được khi `none`.

**Hồ sơ Bộ Công Thương.** Hồ sơ pháp lý theo khách / nút người bán; website theo tên miền. Danh sách
kiểm = bản `moit.checklist.<website_kind>` hiệu lực hôm nay; mỗi mục:
`{"key","title":{"en","vi"},"check":{…},"legal_basis","needs_legal_review"}` với `check.type` thuộc tập
cố định trong code: `profile_field` (trường của hồ sơ có giá trị), `page` (trang `page_kind` gắn với
website, `Resource.public` và có `url` — tra qua `ResourceDirectory`), `document` (văn bản `kind` có bản
hiện hành), `kyc` (hồ sơ pháp lý có KYC `verified`), `policy` (khoá policy có giá trị, vd
`complaints.contact`), `fact` (qua `ComplianceFacts`; chưa ai đăng ký → `unknown`), `manual`. Kết quả
từng mục: `met` / `missing` / `unknown` / `manual`. `autoxide doctor` có check `compliance.moit_readiness`
(duyệt mọi khách qua API của bước 33): website `not_submitted` thiếu mục → `warning`, kèm gợi ý gọi
`GET …/websites/{id}/checklist`; khách có `data_residency = VN` mà nơi đặt không rõ / khác → `warning`.
**Xuất bộ hồ sơ** (`GET …/dossier`): `form` (các trường tờ khai điền từ hồ sơ pháp lý, website, KYC),
`documents` (nháp đề án cung cấp dịch vụ và quy chế dạng Markdown, điền từ cấu hình thật: chế độ dòng
tiền của bước 36 nếu có, kênh khiếu nại, hạn xử lý từ `cases.deadlines`, danh sách trang bắt buộc kèm
URL), `checklist`, `generated_at`, `disclaimer` (cần người có chuyên môn pháp lý duyệt). Trạng thái
logo cập nhật tay (`POST …/status`) — `logo_snippet` chỉ nhận `<a href="https://online.gov.vn/…"><img
src="https://…"></a>` (parse, dựng lại từ `link_url` + `image_url`; HTML khác → `compliance.logo_snippet_invalid`;
miền cho phép là dữ liệu trong `moit.checklist.*`).

**"Chặn bán" khi website chưa thông báo (A-3.5).** **Cần đối chiếu** NĐ 52/2013/NĐ-CP sửa bởi NĐ
85/2021/NĐ-CP và Luật Thương mại điện tử 2025: bán trước khi thông báo website với Bộ Công Thương bị **cấm**
hay chỉ **bị phạt**. Nếu cấm: thêm cổng chặn đặt hàng (`compliance.website_not_notified`, 403) gọi từ
checkout của bước 45. Nếu không: nhãn trong danh sách kiểm đổi thành "bắt buộc theo luật", không chặn. Agent
bước 37 ghi kết quả đối chiếu vào DECISIONS trước khi chọn ("Agent tự quyết" #10).

**Văn bản pháp lý có phiên bản.** Văn bản được soạn là trang của `content` (một cách soạn trang);
`POST …/documents/{id}/versions {source_entry_id, locale, effective_from, requires_reacceptance}` gọi
`dyn PublishedEntries::published_snapshot` (content, xem C1) → chép **ảnh chụp bất biến** (title, body,
plain_text, `revision_id`, SHA-256) vào `compliance_document_versions`. `version` = max + 1 cho văn bản
(mọi ngôn ngữ của một lần phát hành cùng số; thêm bản dịch cho số đã có thì cùng `version`). Chấp nhận:
`POST /me/acceptances {version_id, channel, method}` (idempotent theo unique) ghi `sha256` của chính
bản đã hiện cho người dùng. Bản mới có `requires_reacceptance` → `GET /me/pending-acceptances` liệt
kê; `AcceptanceGate` từ chối tới khi chấp nhận lại. Bản không đòi chấp nhận lại vẫn có hiệu lực từ
`effective_from` (người dùng thấy bản mới qua endpoint công khai). `GET …/documents/{id}/acceptance-stats`
trả số người đã chấp nhận theo phiên bản (A-c9).

**Vụ việc (SHTT / hàng giả / khiếu nại người tiêu dùng).** Tiếp nhận (công khai có chống spam
`autoxide_kernel::antispam` như `forms`, hoặc người đăng nhập, hoặc nhân sự nhập từ cơ quan) →
`received`, tính `ack_due_at` / `decide_by` từ `cases.deadlines` (ngày làm việc: bỏ thứ Bảy, Chủ nhật;
ngày lễ là dữ liệu trong cùng quy tắc) → `restrict` (tự động khi `cases.auto_restrict` bật cho
`counterfeit` / `ip_infringement`, hay tay) phát event `compliance.target_restricted {case_id, target}`;
module sở hữu đích nghe và ẩn (C3) → báo bên bị phản ánh (`Resource.owner_user_ids` + quản trị nút
`respondent_org_key`) bằng mail, `awaiting_response` tới `respond_by` → phản hồi (`POST …/responses`, chỉ
bên bị phản ánh) → quyết định (phiên đăng nhập của người có `compliance.cases.handle`) → `upheld` giữ ẩn /
`rejected` phát `compliance.target_restored`. Mọi bước thêm một dòng `compliance_case_entries` (lưu vết).
Người báo tra trạng thái bằng token một lần trả lúc gửi (chỉ lưu băm). Cung cấp thông tin người bán
cho cơ quan: `POST …/disclosures` ghi cơ quan, số văn bản, căn cứ, tên trường đã đưa → event
`compliance.disclosure_recorded` (audit; quản trị khách xem được qua `GET …/disclosures`).

**Dữ liệu cá nhân.** Sổ mục đích (`privacy.default_purposes` + bảng khách) công khai qua
`GET /public/purposes` (minh bạch); sổ đồng ý như A1; người dùng xem / đổi ở `/me/consents`; rút qua
link ký trong mail (`POST /public/consents/withdraw {token}`, token HMAC mang tenant + consent id, hết
hạn 1 năm). Rút đồng ý → event `compliance.consent_withdrawn {consent_id, purpose, scope, channel}`
(không email / số). Xoá theo yêu cầu (event `auth.user_deleted` hay `DELETE …/consents?subject=`) → đặt
`subject_value = NULL`, giữ `subject_key` và bản ghi rút để chứng minh / không gửi lại, xoá hẳn sau
`privacy.consent_proof_days`. Thời hạn lưu: `GET …/retention` gộp `RetentionCatalog` (module khai) với
`privacy.retention_limits`; doctor cảnh báo mục `Basic`/`Sensitive` để `Forever` hay vượt giới hạn. Vị trí
lưu: `GET/PUT …/residency` đọc / đặt `data_residency` qua API kernel (chỉ nhân sự nền tảng đặt). Sự cố:
tạo sự cố → `report_due_at`; job `compliance.incident_deadlines` (15 phút) gửi mail tới
`incident.alert_emails` ở các mốc `remind_at_hours` và phát `compliance.incident_deadline_near`; sổ tay
72 giờ ở `docs/COMPLIANCE*.md`. Sự cố có bản nháp thông báo gửi cơ quan và gửi người bị ảnh hưởng (AI chỉ
là bản nháp, D-109; A-c9), mốc 72 giờ như trên. `GET …/processing-register` xuất sổ hoạt động xử lý (mục
đích, căn cứ, loại dữ liệu, thời hạn, nơi lưu) làm đầu vào cho hồ sơ đánh giá tác động — mẫu hồ sơ cần đối
chiếu.

**Sổ yêu cầu của chủ thể dữ liệu (D-217, A-b3, A-c8).** Yêu cầu đến qua kênh khác (email, thư, điện thoại, cơ
quan) ghi vào `compliance_subject_requests`; `POST …/subject-requests/{id}/fulfil` chạy job xuất / xoá dùng
cơ chế sẵn có; việc vào hộp việc kèm hạn. Hạn trả lời là dữ liệu có ngày hiệu lực (Luật 91/2025/QH15, NĐ
356/2025 — **cần đối chiếu văn bản gốc**). Không áp cho yêu cầu người dùng tự làm trong tài khoản: xuất và
xoá dữ liệu người dùng tự bấm chạy ngay, không qua duyệt (D-217).

**Bảo vệ người tiêu dùng.** Văn bản `general_conditions` (điều kiện giao dịch chung) công khai qua
`/public/documents/general_conditions`, chấp nhận lúc thanh toán (bước 45 gọi `AcceptanceGate`); khiếu
nại = case `consumer_complaint` với hạn từ `cases.deadlines`; `ModerationPolicy` + `moderation.hide_reasons`
(danh sách lý do được ẩn đánh giá: vd `spam`, `illegal_content`, `personal_data`, `unrelated` — cần đối
chiếu) để 45 không ẩn đánh giá tiêu cực hợp lệ; mọi lần ẩn ghi lý do (audit qua event của module sở hữu).

### A4. API REST (`/api/v1/compliance`)

Mọi response theo `Data` / `Page` / `Problem`; danh sách dùng `fetch_page`; bản ghi sửa được trả
`ETag` và nhận `If-Match`; tạo mới trả `Created`. Lệnh ghi nhận `Idempotency-Key`.

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /me/verification` | đăng nhập | mức `none/phone/identity`, số đã che |
| `POST /me/phone-verifications`, `POST /me/phone-verifications/{id}/confirm`, `DELETE /me/phone` | đăng nhập | lỗi `compliance.otp_invalid`, `…otp_attempts_exceeded`, `…phone_not_allowed`, `…phone_in_use`, `sms_disabled` |
| `GET /me/kyc`, `POST /me/kyc`, `POST /me/kyc/sessions` | đăng nhập | KYC cá nhân (người livestream, tiếp thị) |
| `GET /kyc/profiles`, `GET /kyc/profiles/{id}` | `compliance.kyc.read` | lọc `status`, `subject` |
| `POST /kyc/profiles`, `POST /kyc/profiles/{id}/sessions`, `POST /kyc/profiles/{id}/documents` | `compliance.kyc.submit` (hay quản trị nút có `Scope` trên nút đó) | hồ sơ của nút người bán |
| `POST /kyc/profiles/{id}/approve` · `/reject` · `/revoke` | `compliance.kyc.review`, phiên người | `reason_code` bắt buộc khi reject / revoke |
| `POST /kyc/profiles/{id}/request-info` | chưa nêu ("Agent tự quyết" #11) | `{fields, message}` → `needs_info` (A-c9) |
| `POST /kyc/callbacks/{provider}` | công khai, chữ ký | 204; lỗi `identity_callback_invalid` |
| `GET/POST /legal-profiles`, `GET/PATCH /legal-profiles/{id}` | `compliance.legal.read` / `.manage` | `Address` kiểm qua `AdministrativeUnits` |
| `GET /legal-profiles/{id}/template-variables` | `compliance.legal.read` | biến cho mẫu trang của `content` |
| `GET/POST /websites`, `GET/PATCH /websites/{id}` | `compliance.legal.read` / `.manage` | `compliance.domain_taken` |
| `GET /websites/{id}/checklist`, `GET /websites/{id}/dossier` | `compliance.legal.read` | |
| `PUT/DELETE /websites/{id}/pages/{page_kind}` | `compliance.legal.manage` | `{entry_id}`; đích phải là `content.entry` có thật |
| `POST /websites/{id}/status` | `compliance.legal.manage` | `{status, reference, link_url, image_url, snippet}` |
| `GET/POST /documents`, `GET /documents/{id}`, `GET/POST /documents/{id}/versions` | `compliance.documents.read` / `.manage` | `compliance.source_not_published` |
| `GET /documents/{id}/acceptances` | `compliance.documents.read` | bằng chứng, lọc theo subject |
| `GET /documents/{id}/acceptance-stats` | chưa nêu ("Agent tự quyết" #11) | số người đã chấp nhận theo phiên bản (A-c9) |
| `GET /me/pending-acceptances`, `POST /me/acceptances` | đăng nhập | |
| `GET/POST /purposes`, `PATCH /purposes/{key}` | `compliance.privacy.read` / `.manage` | |
| `GET /me/consents`, `PUT /me/consents` | đăng nhập | `{purpose, scope, channel, granted, text, text_version}` |
| `GET /consents`, `DELETE /consents` | `compliance.privacy.manage` | tra / xoá theo `subject` (email, số, user) cho yêu cầu của chủ thể |
| `GET /retention`, `GET /processing-register`, `GET/PUT /residency` | `compliance.privacy.read` / `.manage` (PUT: nhân sự nền tảng) | |
| `GET/POST /subject-requests`, `POST /subject-requests/{id}/fulfil` | chưa nêu ("Agent tự quyết" #11) | sổ yêu cầu của chủ thể qua kênh khác; `fulfil` chạy job xuất / xoá (D-217) |
| `GET/POST /incidents`, `GET/PATCH /incidents/{id}`, `POST /incidents/{id}/entries` | `compliance.incidents.manage` | |
| `GET/POST /cases`, `GET /cases/{id}`, `POST /cases/{id}/entries` | `compliance.cases.read` / `.handle` | |
| `POST /cases/{id}/restrict` · `/restore` · `/decision` | `compliance.cases.handle` (decision: phiên người) | `compliance.case_state_invalid` |
| `GET /respondent/cases`, `POST /cases/{id}/responses` | đăng nhập, là bên bị phản ánh | `compliance.response_closed` |
| `GET/POST /disclosures` | `compliance.disclosures.manage` | |
| `GET /policies`, `PUT /policies/{key}` | `compliance.policies.manage` | |
| `GET /rules`, `POST /rules` | `compliance.rules.read` / `compliance.rules.manage` (nhân sự nền tảng) | nhập bản quy tắc mới; `compliance.rules_invalid` |

**Công khai** (tenant suy từ tên miền, không cần đăng nhập, có `ETag` / cache):
`GET /public/badge` (website của tên miền đang gọi: `kind` `notified`/`registered`, `link_url`,
`image_url`, `snippet`; chưa có → 404 `compliance.website_not_found`), `GET /public/owner` (thông tin
chủ sở hữu cho chân trang), `GET /public/documents/{kind}?locale=` (bản hiện hành + `version`, `sha256`),
`GET /public/purposes`, `GET /public/moderation-reasons`, `POST /public/reports` và `POST
/public/complaints` (chống spam, throttle theo IP), `GET /public/cases/{id}?token=`, `POST
/public/consents/withdraw`.

### A5. Tool MCP · Event · Job · Cấu hình · Permission · Mã lỗi

**Tool MCP** (cùng permission, cùng service): `get_verification_status`, `list_kyc_profiles` (read),
`get_website_checklist`, `get_website_dossier` (read), `list_documents`, `publish_document_version`,
`list_pending_acceptances` (read), `list_cases`, `get_case` (read), `add_case_note`, `restrict_case_target`,
`list_purposes` (read), `open_incident`, `add_incident_entry`. **Không** có tool cho duyệt KYC và quyết
định vụ việc (cần phiên người — REST với API key cũng không làm được, nên không vượt luật 8).

**Event** (payload `{before, after}`, chỉ id / trạng thái, không email, số điện thoại, số giấy tờ):
`compliance.kyc_submitted`, `…kyc_verified`, `…kyc_rejected`, `…kyc_expired`, `…kyc_revoked`,
`…phone_verified`, `…phone_removed`, `…legal_profile_updated`, `…website_created`, `…website_updated`,
`…website_status_changed`, `…document_version_published`, `…document_accepted`, `…consent_granted`,
`…consent_withdrawn`, `…purpose_updated`, `…policy_updated`, `…case_opened`, `…case_updated`,
`…target_restricted`, `…target_restored`, `…case_decided`, `…disclosure_recorded`, `…incident_opened`,
`…incident_updated`, `…incident_deadline_near`, `…rules_imported`. Nghe: `auth.user_deleted`
(`privacy::USER_DELETED`) để xoá / ẩn danh dữ liệu của người đó (giữ bằng chứng chấp nhận theo
`privacy.consent_proof_days`, ghi lý do giữ trong `PersonalData` export).

**Job / lịch:** `compliance.kyc_process` (callback), `compliance.kyc_poll`, lịch hằng ngày
`compliance.kyc_expiry`, `compliance.kyc_documents_purge`, `compliance.challenges_purge`,
`compliance.consent_proof_purge`, `compliance.rules_refresh` (nạp gói `vn` đóng trong crate nếu có
`version` mới, idempotent); lịch giờ `compliance.case_deadlines` (quá hạn → event + mail); lịch 15 phút
`compliance.incident_deadlines`. Task: `compliance.import_rules --file <json>`,
`compliance.export_dossier --website <id>`.

**Setting** (cấp máy chủ; cấp khách nằm ở `compliance_policies`):
`AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS` (300), `…_OTP_MAX_ATTEMPTS` (5), `…_OTP_PER_PHONE_PER_HOUR` (3),
`…_OTP_PER_USER_PER_DAY` (10), `…_KYC_DOCUMENT_RETENTION_DAYS` (30), `…_KYC_CALLBACK_RETENTION_DAYS`
(90), `…_PUBLIC_REPORTS_PER_IP_PER_HOUR` (5), `…_CASE_TRACKING_URL` (trang giao diện tra vụ việc),
`…_CONSENT_WITHDRAW_URL`.

**Permission:** `compliance.kyc.read`, `compliance.kyc.submit`, `compliance.kyc.review`,
`compliance.legal.read`, `compliance.legal.manage`, `compliance.documents.read`,
`compliance.documents.manage`, `compliance.privacy.read`, `compliance.privacy.manage`,
`compliance.incidents.manage`, `compliance.cases.read`, `compliance.cases.handle`,
`compliance.disclosures.manage`, `compliance.policies.manage`, `compliance.rules.read`,
`compliance.rules.manage`.

**Mã lỗi** (EN ở chỗ tạo lỗi; VI trong `modules/compliance/locales/vi.json`):

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `compliance.kyc_required` | 403 | Cần xác thực danh tính trước khi {activity}. |
| `compliance.kyc_not_found` | 404 | Không có hồ sơ định danh này. |
| `compliance.kyc_state_invalid` | 409 | Hồ sơ định danh đang ở trạng thái {status}, không làm được việc này. |
| `compliance.verification_required` | 403 | Tài khoản cần xác thực {level} trước khi đăng nội dung ở đây. |
| `compliance.otp_invalid` | 422 | Mã xác thực không đúng hoặc đã hết hạn. |
| `compliance.otp_attempts_exceeded` | 429 | Nhập sai quá số lần cho phép; hãy xin mã mới. |
| `compliance.phone_not_allowed` | 422 | Số điện thoại phải là số di động tại Việt Nam. |
| `compliance.phone_in_use` | 409 | Số điện thoại này đã xác thực cho tài khoản khác. |
| `compliance.acceptance_required` | 403 | Cần đồng ý {document} phiên bản {version} trước. |
| `compliance.document_not_found` | 404 | Không có văn bản này. |
| `compliance.source_not_published` | 422 | Trang nguồn chưa được đăng; hãy đăng trang trước khi phát hành phiên bản. |
| `compliance.profile_not_found` | 404 | Không có hồ sơ pháp lý này. |
| `compliance.website_not_found` | 404 | Không có website này. |
| `compliance.domain_taken` | 409 | Tên miền này đã có hồ sơ website. |
| `compliance.logo_snippet_invalid` | 422 | Đoạn mã logo không đúng dạng do Bộ Công Thương cấp. |
| `compliance.case_not_found` | 404 | Không có vụ việc này. |
| `compliance.case_state_invalid` | 409 | Vụ việc đang ở trạng thái {status}, không làm được việc này. |
| `compliance.response_closed` | 409 | Đã hết hạn phản hồi vụ việc này. |
| `compliance.purpose_unknown` | 422 | Không có mục đích xử lý `{purpose}`. |
| `compliance.consent_not_found` | 404 | Không có bản ghi đồng ý này. |
| `compliance.hide_reason_not_allowed` | 422 | Lý do `{reason}` không được phép dùng để ẩn đánh giá. |
| `compliance.incident_not_found` | 404 | Không có sự cố này. |
| `compliance.rules_invalid` | 422 | Bản quy tắc không hợp lệ: {detail}. |
| `compliance.link_invalid` | 400 | Link đã hết hạn hoặc không hợp lệ. |
| `compliance.website_not_notified` | 403 | Website chưa thông báo với Bộ Công Thương nên chưa được bán. — **chỉ thêm khi** đối chiếu kết luận "cấm" (A3, "Chặn bán") |

---

## Phần B — module `notifications`

### B1. Hợp đồng `autoxide_notifications::api` (phác thảo)

```rust
pub enum Category { Security, Transactional, Marketing }   // Security: không tắt được
pub enum Channel { Email, Sms, Zns, WebPush, InApp, MobilePush /* D-220 */ }
pub struct NotificationKind { pub key: &'static str /* "orders.order_shipped" */, pub category: Category,
    pub channels: &'static [Channel], pub template: NotificationTemplate,
    pub purpose: Option<&'static str> /* Marketing: khoá mục đích trong sổ đồng ý, mặc định "marketing" */ }
/// Như `MailTemplate`: câu EN trong code, bản dịch ở `locales/<lang>.json` của crate khai báo với khoá
/// `<key>.title`, `<key>.body`, `<key>.short` (SMS / push); `cargo xtask check` kiểm đủ.
pub struct NotificationTemplate { /* key, title, body, short, zns_params: &'static [&'static str] */ }
pub enum Recipient { User(Uuid), Email(String), Phone(PhoneNumber) }
pub struct Notification { pub kind: &'static str, pub recipient: Recipient, pub params: Value /* chỉ chữ hiển thị */,
    pub link: Option<String>, pub locale: Option<String>, pub dedupe_key: String /* bắt buộc, vd id event */ }
pub struct Outcome { pub message_ids: Vec<Uuid>, pub skipped: Vec<(Channel, SkipReason)> }
#[async_trait] pub trait Notifier: Send + Sync {
    /// Gọi trong `Module::start`; khai trùng key → lỗi.
    fn declare(&self, kind: NotificationKind) -> Result<(), Error>;
    /// Ghi tin + job trong transaction riêng của `notifications`; KHÔNG gọi trong transaction ghi của
    /// module khác (gọi từ handler event hay job; cùng `dedupe_key` → không tạo tin thứ hai).
    async fn notify(&self, tenant: TenantId, notification: Notification) -> Result<Outcome, Error>;
}
#[async_trait] pub trait ZnsSender: Send + Sync {   // bộ nối Zalo cài, đăng ký trước notifications
    fn provider(&self) -> &'static str;
    async fn send(&self, tenant: TenantId, to: &PhoneNumber, template_id: &str, params: &Value, reference: &str) -> Result<String, Error>;
}
```

- **Schema biến** (D-206): mỗi loại thông báo khai cùng chỗ với `NotificationKind` một JSON Schema cho biến
  và một bộ dữ liệu mẫu để xem trước (mục 2).
- **Kênh `MobilePush`** (D-220): trait `MobilePushSender`; bộ cài đặt cho Firebase Cloud Messaging và Apple
  Push Notification service là module riêng, cần khoá của nhà cung cấp — khi đó dừng lại hỏi chủ dự án; bản
  giả cho test có sẵn.

### B2. Bảng dữ liệu (cấp khách hàng; migration `m0001_create_notifications` có `down`)

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `notifications_messages` | `id`, `tenant_id`, `kind`, `category`, `channel`, `user_id` NULL, `address` (email / số / endpoint id; xoá sau hạn lưu), `locale`, `params` JSON, `status` (`queued`/`deferred`/`sent`/`failed`/`skipped`/`canceled`), `skip_reason` (`no_consent`/`consent_withdrawn`/`preference_off`/`no_address`/`channel_disabled`/`quiet_hours_expired`/`invitation_used`/`''`), `not_before`, `attempts`, `provider_reference`, `error_code`, `dedupe_key`, `created_at`, `sent_at` | unique (`tenant_id`,`dedupe_key`,`channel`); index (`tenant_id`,`user_id`,`id`), (`tenant_id`,`status`,`not_before`) |
| `notifications_inbox` | `id`, `tenant_id`, `user_id`, `kind`, `title`, `body`, `link`, `read_at`, `created_at` | index (`tenant_id`,`user_id`,`id`), (`tenant_id`,`user_id`,`read_at`) |
| `notifications_preferences` | `tenant_id`, `user_id`, `target` (`kind:<key>` hay `category:<tên>`), `channel`, `enabled`, `updated_at` | PK (`tenant_id`,`user_id`,`target`,`channel`) |
| `notifications_templates` | `tenant_id`, `kind`, `channel`, `locale`, `title`, `body`, `short`, `zns_template_id`, `format` (`plain` mặc định — giữ mẫu cũ / `template`), `subject`, `body_html` NULL, `body_text`, `version`, `published_at`, `updated_by`, `updated_at` (D-206) | PK (`tenant_id`,`kind`,`channel`,`locale`) — ghi đè mẫu mặc định. Bản nháp và bản đang dùng tách nhau; mỗi lần xuất bản tăng `version`, giữ 20 bản gần nhất để quay lại (D-206) |
| `notifications_push_subscriptions` | `id`, `tenant_id`, `user_id`, `endpoint`, `endpoint_key` (SHA-256), `p256dh`, `auth_secret` (mã hoá AEAD của bước 33), `user_agent`, `failures`, `created_at`, `last_used_at` | unique (`tenant_id`,`endpoint_key`); index (`tenant_id`,`user_id`) |
| `notifications_invitations` | `tenant_id`, `channel`, `subject_key` (SHA-256 số / email), `sent_at` | PK (`tenant_id`,`channel`,`subject_key`) — "chỉ một tin mời đăng ký đầu tiên" |
| `notifications_device_tokens` (D-220) | `id`, `tenant_id`, `user_id`, `platform` (`ios`/`android`), `token` (mã hoá AEAD của bước 33), `app_id`, `locale`, `created_at`, `last_used_at`, `failures` | unique (`tenant_id`, băm `token`) |

Cùng `notifications_events`, `notifications_jobs`, `notifications_throttle`.

### B3. Luồng gửi

`notify` → tra loại đã khai → chọn kênh = `kind.channels` ∩ ưu tiên người dùng (`Security` bỏ qua
ưu tiên) ∩ kênh có địa chỉ (email từ `dyn Users`; số từ `AccountVerification::verified_phone`; push từ
`notifications_push_subscriptions`; in-app luôn có khi `User`) → với `Marketing`: `ConsentLedger::is_granted
(subject, {purpose, scope:"", channel})` — không có → ghi `skipped/no_consent`, không gửi → mỗi kênh một dòng
`notifications_messages` + job `notifications.deliver {message_id}` (payload chỉ id) trong một transaction.
Worker `deliver`: đọc lại dòng; với `Marketing` **kiểm lại đồng ý ngay trước khi gọi nhà cung cấp** (rút
giữa chừng → `skipped/consent_withdrawn`); kiểm khung giờ `notifications.send_windows` (ngoài khung → `deferred`,
đặt lại job `run_at` = lần mở khung kế tiếp; quá 24 giờ chưa gửi được → `skipped/quiet_hours_expired`); thêm
nhãn quảng cáo từ `notifications.marketing_labels` (vd tiền tố "QC" cho SMS — cần đối chiếu); gửi qua
`Mailer` / `SmsSender` / `ZnsSender` / web push / ghi `notifications_inbox`. Lỗi tạm thời → retry của job
queue; vĩnh viễn → `failed` + `error_code`. Email quảng cáo có `List-Unsubscribe` và link rút đồng ý (token ký
mang tenant, kind, channel). Nghe `compliance.consent_withdrawn` → huỷ (`canceled`) tin quảng cáo `queued` /
`deferred` của subject đó. Tin mời đăng ký nhận quảng cáo (kind do module khai với
`Category::Marketing` + cờ `invitation`) chỉ gửi khi chưa có dòng trong `notifications_invitations`.

`notifications.send_windows` (dữ liệu, giờ địa phương bằng **độ lệch phút cố định** `utc_offset_minutes:
420` — Việt Nam không có giờ mùa hè, khỏi kéo thư viện múi giờ): `{"marketing":{"sms":{"from":"07:00",
"to":"22:00"},"zns":{…},"email":null,"push":null},"max_marketing_per_day":{"sms":null}}`.

Web push: VAPID (RFC 8292) + mã hoá nội dung (RFC 8291); khoá VAPID dẫn xuất từ
`SecretKey::derive("notifications.vapid")` (không thêm secret mới); `GET /public/web-push-key` trả khoá
công khai. Endpoint trả 404 / 410 → xoá subscription.

Đẩy di động (`MobilePush`, D-220): token thiết bị trong `notifications_device_tokens`; nhà cung cấp báo token
hết hiệu lực → xoá. Cùng luật như web push: tuỳ chọn nhận theo loại, tin quảng cáo cần đồng ý, khung giờ gửi.

**Dựng mẫu khi gửi (D-206).**

- Dựng mẫu ở worker `deliver`, **ngoài transaction**; lỗi dựng (mẫu hỏng sau khi lưu do dữ liệu bất
  thường) → dùng mẫu mặc định trong code, ghi `error_code = template_fallback`, cảnh báo cho quản trị —
  không bỏ tin giao dịch hay bảo mật.
- Kênh SMS / ZNS / push dùng phần `short` của cùng bộ dựng (không HTML); ZNS vẫn đi theo mẫu đã duyệt của
  Zalo, bộ dựng chỉ điền `zns_params`.
- Mẫu tin **quảng cáo** không thể bỏ chân thư huỷ nhận và nhãn quảng cáo: bộ dựng tự chèn, mẫu không gỡ
  được. Không chèn điểm ảnh theo dõi hay link theo dõi vào mẫu mặc định; khách bật đo mở thư thì phải có
  căn cứ xử lý trong sổ của `compliance` (Luật 91/2025 — cần đối chiếu).

**Báo cáo đã lưu (ưu tiên thấp, A-b10).** Báo cáo đã lưu và lịch gửi qua email, dùng xuất chung của bước 51
(D-213).

### B4. API, MCP, event, job, cấu hình, permission, lỗi

**REST `/api/v1/notifications`:**

- `GET /kinds` (đăng nhập: loại, nhóm, kênh — cho trang tuỳ chọn); `GET /kinds/{kind}/schema` (schema biến
  và dữ liệu mẫu, D-206).
- `GET /me/inbox` (Page), `GET /me/inbox/unread-count`, `POST /me/inbox/{id}/read`, `POST /me/inbox/read-all`;
  `GET /me/preferences`, `PUT /me/preferences` (`notifications.preference_locked` khi tắt `Security`).
- `POST /me/push-subscriptions`, `DELETE /me/push-subscriptions/{id}`; `POST /me/device-tokens`, `DELETE
  /me/device-tokens/{id}` (D-220).
- Mẫu (`notifications.templates.manage`): `GET /templates`, `PUT/DELETE /templates/{kind}/{channel}/{locale}`;
  với cùng đường dẫn mẫu (D-206): `PUT …/draft`, `POST …/publish`, `POST …/rollback {version}`, `POST
  …/preview` (dựng bằng dữ liệu mẫu của loại hay dữ liệu người gọi gửi — chỉ khoá có trong schema), `POST
  …/test-send {to}` (gửi thử tới **địa chỉ đã xác minh của chính người gọi**, không tới địa chỉ tuỳ ý — chống
  dùng làm công cụ gửi thư rác; có throttle).
- `GET /branding`, `PUT /branding` (logo, màu, chân thư theo nút; `notifications.branding.manage`, D-206).
- `POST /messages` (`notifications.send`; kind `Marketing` cần thêm `notifications.send_marketing`), `GET
  /messages`, `GET /messages/{id}` (`notifications.messages.read`, có dữ liệu cá nhân).
- Công khai: `POST /public/unsubscribe {token}`, `GET /public/web-push-key`.

**MCP:** `list_kinds` (read), `send_notification`, `list_messages` (read), `list_my_inbox` (read),
`mark_inbox_read`, `preview_template` (`read_only`), `update_template_draft` (D-206). Xuất bản mẫu qua REST
hay tool `publish_template` có `approval_required` cho key `agent` (P12 bước 51). AI gợi ý mẫu (D-109) chỉ là
bản nháp.

**Event:** `notifications.template_updated`, `…template_deleted`, `…template_published`,
`…template_rolled_back`, `…branding_updated` (D-206), `…preferences_updated`, `…push_subscription_added`,
`…push_subscription_removed` — **không** phát event cho từng tin (audit phình; nhật ký gửi đã là
`notifications_messages`).

**Job:** `notifications.deliver`; lịch hằng ngày `notifications.purge` (xoá `address` / `params` của tin quá
`RETENTION_DAYS`, xoá inbox quá hạn).

**Setting:** `AUTOXIDE_NOTIFICATIONS_RETENTION_DAYS` (90), `…_INBOX_RETENTION_DAYS` (180),
`…_UNSUBSCRIBE_URL`, `…_WEB_PUSH_CONTACT` (`mailto:` cho VAPID), `…_PER_RECIPIENT_PER_HOUR` (20).

**Permission:** `notifications.send`, `notifications.send_marketing`, `notifications.messages.read`,
`notifications.templates.manage`, `notifications.branding.manage` (D-206).

**Mã lỗi:** `notifications.kind_unknown` (422, "Không có loại thông báo `{kind}`."),
`notifications.template_invalid` (422, "Mẫu thông báo thiếu tham số {params}."),
`notifications.message_not_found` (404, "Không có tin này."), `notifications.preference_locked` (422,
"Không tắt được thông báo bảo mật."), `notifications.push_subscription_invalid` (422, "Đăng ký nhận thông
báo đẩy không hợp lệ."), `notifications.link_invalid` (400, "Link đã hết hạn hoặc không hợp lệ."),
`notifications.zns_disabled` (503, "Máy chủ chưa cấu hình Zalo ZNS."). Lỗi lưu mẫu `template.*` của kernel ở
mục 2 (D-206).

---

## Phần C — sửa module và kernel có sẵn

- **C1 `content`:** (a) mẫu trang bắt buộc trong `modules/content/templates/legal/<key>.<en|vi>.json`
  (thân ProseMirror theo allowlist hiện có, chỗ điền `{{owner.legal_name}}`, `{{contact.email}}`,
  `{{policy.return_days}}`…): `owner_info`, `privacy_policy`, `return_policy`, `shipping_policy`,
  `payment_policy`, `inspection_policy`, `complaint_policy`, `terms_of_use`, `general_conditions`;
  sàn thêm `platform_rules`, `seller_agreement`. Đoạn đầu mỗi mẫu: "Nội dung mẫu — cần người có chuyên
  môn pháp lý duyệt trước khi dùng". Endpoint `GET /api/v1/content/templates` và `POST
  /api/v1/content/templates/{key}/drafts {locale, variables, parent_id?}` (`content.write`) tạo trang
  nháp; thay biến chỉ trong text node (không chèn được thẻ); thiếu biến bắt buộc → 422
  `content.template_variables_missing` liệt kê đủ một lần; `content.template_not_found` (404). (b)
  `pub mod api` với trait `PublishedEntries { async fn published_snapshot(&self, tenant, entry_id) ->
  Result<Option<PublishedSnapshot>, Error> }` (title, locale, body JSON, plain_text, `revision_id`,
  `published_at`), công bố trong `start`. (c) `ai_assisted` (C4). (d) nghe
  `compliance.target_restricted` / `…target_restored` cho `content.entry:<id>`: cột mới
  `content_published.restricted_at`; trang công khai trả **451** `content.restricted` ("Nội dung tạm ẩn do
  có phản ánh vi phạm."), sitemap / feed / tìm kiếm bỏ qua; idempotent.
- **C2 `comments` (D-202):** kernel đã có `autoxide_kernel::access::PostingVerifier` (`is_verified(user_id)`)
  và chỗ giữ `PostingVerification`; `comments` hỏi nó cho các loại đích trong
  `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES` (khách vãng lai luôn bị từ chối ở đó, lỗi
  `comments.verification_required`). Chưa ai đăng ký thì mọi tài khoản đăng nhập tính là đã xác thực.
  Bước này: `compliance` đăng ký `context.posting_verifier(…)` trong `start`, cài bằng
  `AccountVerification::level` ≥ mức `posting.comments` của khách (khi bước 33 đã có, thêm tham số khách
  vào trait của kernel bằng một hàm mới có cài đặt mặc định — D-038). `comments` **không**
  `require::<dyn PostingGate>` và không phải dời thứ tự đăng ký; `posting.comments` khác `none` thì
  `compliance` coi mọi loại đích của `comments` là "chỉ tài khoản đã xác thực" — ghi trong
  `docs/COMMENTS.md` (+ `.vi.md`) cách hai cấu hình kết hợp. Nghe `target_restricted` / `restored` cho
  `comments.comment:<id>` → ẩn / hiện lại (giữ trạng thái cũ để khôi phục đúng).
- **C3 `newsletter`:** dời đăng ký sau `compliance`; mọi đường xác nhận (link, danh sách không double
  opt-in, nhập có `consent_source`, trang tuỳ chọn) gọi `ConsentLedger::grant` **trước** transaction của
  mình (`subject = Email`, `purpose = "marketing"`, `scope = "newsletter.list:<id>"`, `channel = "email"`,
  evidence = JSON `consent` đang lưu, `source_ref = "newsletter.subscription:<subscriber>:<list>:<lần>"`);
  mọi đường huỷ gọi `withdraw` trước transaction. Lúc gửi chiến dịch, mỗi lô lọc người nhận bằng
  `granted_many` (thiếu → delivery `skipped`, lý do `consent_withdrawn`). Nghe
  `compliance.consent_withdrawn` → `ConsentLedger::get` lấy email → huỷ các đăng ký khớp (không gọi lại
  ledger nếu đã `unsubscribed`). Thêm task `newsletter.backfill_consents` ghi sổ cho đăng ký `confirmed` có
  sẵn (idempotent theo `source_ref`). Chiến dịch dùng bộ dựng mẫu (D-206): thêm `format` (`static` mặc định
  — hành vi cũ / `template`); biến được phép: thông tin người nhận đã có (`subscriber.name`,
  `subscriber.email` che bớt), danh sách, link huỷ — không gì khác. Dựng từng người nhận trong job gửi theo
  lô, dùng chung một mẫu đã biên dịch. Endpoint chiến dịch hiện có giữ nguyên hành vi (D-059); thêm trường
  mới, không đổi trường cũ.
- **C4 AI (Luật 134/2025):** `content_entries`, `content_published` thêm `ai_assisted BOOL DEFAULT false`;
  snapshot revision thêm trường (serde default `false`); `PATCH /entries/{id}` nhận `ai_assisted` — đặt
  `true` khi người dùng áp gợi ý từ `seo-suggestions` hay sửa từ bản AI; đặt lại `false` cần
  `content.publish`. Response quản trị và công khai trả `ai_assisted` (tài liệu OpenAPI nêu Luật 134/2025).
  `media_items.alt_ai_assisted` tương tự cho alt text gợi ý. `ai_generated` / `ai_reviewed` giữ nguyên.
- **C5 `Module::retention`:** khai cho `auth` (phiên, token), `audit`, `webhooks` (lần gửi), `forms`,
  `newsletter`, `comments` (bình luận, dấu vết IP), `media` (thùng rác), `compliance`, `notifications` —
  `keep` đọc từ setting đang có (không thêm setting mới).
- **C6 `crates/app/src/modules.rs`:** thêm `compliance`, `notifications`; dời `newsletter` xuống sau
  (`comments` không phải dời — D-202); cập nhật bài `worker_chay_lich_dinh_ky_cua_module` (số lịch).

## Thứ tự làm

| # | Commit | Mục ROADMAP |
|---|---|---|
| 1 | Kernel `identity.rs`, `sms.rs` (`PhoneNumber` có test chuẩn hoá số VN), `StartContext::optional`, `Module::retention` + `RetentionCatalog`, migration cư trú dữ liệu, lỗi kernel; testkit `FakeIdentityVerifier`, `FakeSmsSender`; `MODULE_GUIDE` (EN + VI) | KYC, SMS, dữ liệu cá nhân |
| 2 | Crate `compliance` khung: `lib.rs`, `api.rs` đủ trait A1, permission, event, `locales/vi.json`, ba file migration đặt sẵn tên, `compliance_rules` + gói `vn` + `ComplianceRules` + task / job nạp | nền chung |
| 3 | KYC: hồ sơ, attempt, callback, tài liệu thủ công, `KycGate`, job hết hạn, `needs_info` + `request-info` (A-c9) | KYC |
| 4 | Xác thực số điện thoại + `AccountVerification` + `PostingGate` + đăng ký `PostingVerifier` của kernel cho `comments`, `docs/COMMENTS*.md` (D-202) | NĐ 147, SMS |
| 5 | `content`: `pub mod api` `PublishedEntries`, mẫu trang EN + VI, endpoint mẫu | Trang bắt buộc |
| 6 | Hồ sơ pháp lý, website, trang gắn website, danh sách kiểm, `ComplianceFacts`, doctor, dossier, logo, endpoint công khai; đối chiếu "chặn bán" rồi làm cổng hay nhãn (A-3.5) | Hồ sơ Bộ Công Thương |
| 7 | Văn bản có phiên bản, chấp nhận, `AcceptanceGate`, `/public/documents`, `acceptance-stats` (A-c9) | Văn bản pháp lý |
| 8 | Mục đích, sổ đồng ý, `ConsentLedger`, rút qua link, thời hạn lưu (`GET /retention` + khai của module, C5), cư trú dữ liệu, sự cố 72 giờ (kèm bản nháp thông báo, A-c9), sổ xử lý, sổ yêu cầu của chủ thể (D-217); `docs/COMPLIANCE*.md` | Dữ liệu cá nhân |
| 9 | Vụ việc + phản hồi + quyết định + công khai + disclosures; `content` / `comments` nghe ẩn / hiện | Báo vi phạm |
| 10 | `ModerationPolicy`, `moderation.hide_reasons`, `general_conditions`, case `consumer_complaint` | Người tiêu dùng |
| 11 | `ai_assisted` cho `content`, `alt_ai_assisted` cho `media` | Nhãn AI |
| 12 | Crate `notifications`: khai loại, `Notifier`, email + in-app + SMS + ZNS (trait) + web push, ưu tiên, mẫu theo khách, khung giờ, tin mời, `docs/NOTIFICATIONS*.md` | `notifications` |
| 13 | `newsletter` dùng sổ đồng ý (dời thứ tự, backfill) | `newsletter` dùng chung sổ |
| 14 | Kernel `template` (mục 2); `notifications` mẫu động: schema biến, nháp / xuất bản / quay lại, xem trước, gửi thử, thương hiệu theo nút; `newsletter` chiến dịch `format = template` (D-206) | Mẫu mail và thông báo động |
| 15 | Kênh `MobilePush`: trait `MobilePushSender` + bản giả, `notifications_device_tokens`, endpoint token thiết bị (D-220) | Đẩy di động |
| 16 | Báo cáo đã lưu, lịch gửi qua email (ưu tiên thấp, A-b10; cần xuất chung của bước 51, D-213) | `notifications` |
| 17 | `openapi.json`, `.env.example`, `CHANGELOG`, ROADMAP đánh dấu, DECISIONS (mục dưới) | — |

**Chia nhóm song song (theo skill `split-work`):** commit 1–2 do agent chính làm trước và merge (mọi
nhóm cần trait và khung crate; file dùng chung `lib.rs`, `permissions.rs`, `events.rs`, `locales/vi.json`,
`modules.rs` do agent chính giữ — nhóm gửi phần thêm, agent chính gộp). Sau đó bốn nhóm, mỗi nhóm một
worktree: **N1** KYC + số điện thoại + cổng (3, 4); **N2** content mẫu + hồ sơ Bộ Công Thương + văn bản
(5, 6, 7 — 7 cần `PublishedEntries` của 5 trong cùng nhóm); **N3** quyền riêng tư + vụ việc + người tiêu
dùng + nhãn AI (8, 9, 10, 11); **N4** `notifications` + `newsletter` (12, 13, 14, 15 — dùng `ConsentLedger`
của khung; test ban đầu với bản cài trong bộ nhớ, đổi sang thật khi N3 merge; kernel `template` của 14 là file
dùng chung, agent chính gộp). Commit 16 làm sau cùng, khi bước 51 đã có xuất chung. Mỗi nhóm một file migration
đã đặt tên sẵn ở commit 2 (`m0001` N1, `m0002` N2, `m0003` N3) để không đụng nhau.

## Test bắt buộc

Mọi test dùng `TestDatabase::run`, gọi REST / MCP qua `TestApp::builder(test)`, chạy trên SQLite và
Postgres; fake qua `TestAppBuilder::provide::<dyn IdentityVerifier>` / `<dyn SmsSender>`.

- Kernel: `so_viet_nam_chuan_hoa_ve_e164` (`0912 345 678`, `84912…`, `+84 912…`, số cố định / thiếu số bị
  từ chối), `so_dien_thoai_che_khi_ghi_log`, `optional_tra_none_khi_chua_ai_cong_bo`,
  `catalog_thoi_han_luu_gom_moi_module`.
- KYC: `kyc_qua_callback_thanh_verified`, `callback_sai_chu_ky_bi_tu_choi_va_khong_ghi`,
  `callback_nhan_hai_lan_chi_xu_ly_mot`, `kyc_khong_luu_so_giay_to_nguyen_van`,
  `kyc_trung_giay_to_cho_duyet_tay`, `duyet_kyc_bang_api_key_bi_tu_choi`, `kyc_het_han_thanh_expired`,
  `cong_chan_ban_truoc_ngay_hieu_luc_cho_qua_sau_ngay_thi_chan` (đổi thời điểm bằng bản quy tắc trong test),
  `tai_lieu_kyc_thu_cong_bi_xoa_sau_han`.
- Số điện thoại: `otp_dung_mot_lan` (**đồng thời**: hai lần xác nhận cùng mã song song → đúng một thành
  công), `otp_sai_qua_so_lan_bi_khoa`, `otp_khong_nam_trong_job_va_log`, `mot_so_khong_xac_thuc_hai_tai_khoan`,
  `binh_luan_bi_chan_khi_khach_doi_xac_thuc_so`, `khach_vang_lai_bi_chan_khi_doi_xac_thuc`,
  `comments_dung_trang_thai_xac_thuc_that` (đăng ký bộ hỏi thật, người chưa xác thực số bị từ chối — D-202).
- Bộ Công Thương: `danh_sach_kiem_bao_thieu_trang_va_ma_so_thue`, `danh_sach_kiem_doi_theo_phien_ban_quy_tac`,
  `fact_chua_dang_ky_la_unknown`, `snippet_logo_la_bi_tu_choi` (script, `onerror`, miền khác),
  `badge_cong_khai_theo_ten_mien`, `doctor_canh_bao_website_thieu_muc`, `dossier_dien_tu_ho_so_that`.
- Văn bản: `phien_ban_la_anh_chup_khong_doi_khi_trang_sua`, `chap_nhan_hai_lan_khong_tao_hai_dong`,
  `ban_moi_doi_chap_nhan_lai_thi_cong_chan`, `nguon_chua_dang_bi_tu_choi`.
- Đồng ý: `grant_idempotent_theo_source_ref`, `rut_o_marketing_chan_moi_kenh`, `rut_qua_link_ky`,
  `xoa_nguoi_dung_giu_ban_ghi_rut_khong_giu_email`, **đồng thời** `grant_va_withdraw_song_song_khong_mat_dong`.
- Vụ việc: `bao_cong_khai_co_chong_spam`, `an_dich_roi_khoi_phuc_khi_bac` (content trả 451 rồi 200;
  comment ẩn rồi hiện), `nhan_lai_event_an_khong_hong`, `phan_hoi_sau_han_bi_tu_choi`,
  `han_xu_ly_tinh_ngay_lam_viec`, `disclosure_ghi_audit_khong_ghi_gia_tri`.
- Sự cố: `han_72_gio_tinh_tu_luc_phat_hien`, `nhac_han_dung_moc_khong_nhac_trung`.
- Nhãn AI: `ai_assisted_tra_ve_cong_khai`, `bo_ai_assisted_can_content_publish`.
- Notifications: `tin_quang_cao_khong_dong_y_bi_skip`, `rut_dong_y_giua_chung_huy_tin_dang_cho`,
  `ngoai_khung_gio_doi_toi_7h` (thời gian giả), `tin_moi_dang_ky_chi_mot_lan`,
  `thong_bao_bao_mat_khong_tat_duoc`, `dedupe_key_chong_gui_trung` (**đồng thời**: hai `notify` song song
  cùng khoá → một tin), `push_410_xoa_subscription`, `mau_cua_khach_ghi_de_mau_mac_dinh`,
  `payload_job_chi_co_id`; đẩy di động (D-220): `token_het_hieu_luc_bi_xoa`,
  `tin_quang_cao_khong_gui_khi_chua_dong_y` (kênh di động).
- Mẫu động (D-206): `mau_nguoi_dung_khong_doc_duoc_bien_ngoai_schema`,
  `mau_vong_lap_vo_han_bi_cat_theo_gioi_han`, `gia_tri_chen_vao_html_luon_duoc_thoat`,
  `mau_quang_cao_luon_co_chan_thu_huy_nhan`, `luu_mau_thieu_bien_bat_buoc_bi_tu_choi_mot_lan`,
  `gui_thu_chi_toi_dia_chi_cua_nguoi_goi`, `mau_hong_luc_gui_dung_mau_mac_dinh`,
  `xuat_ban_va_quay_lai_phien_ban`, `ban_tin_ca_nhan_hoa_moi_nguoi_nhan`,
  `html_dung_tren_mau_mjml_va_co_ban_chu_thuan`.
- Newsletter: `xac_nhan_ghi_so_dong_y`, `rut_o_compliance_huy_dang_ky_newsletter`,
  `chien_dich_bo_qua_nguoi_da_rut`, `backfill_chay_lai_khong_trung`.
- **Rò rỉ giữa khách** (bài chung của testkit bước 33) cho mọi bảng của `compliance` và
  `notifications`; thêm: `badge_khach_a_khong_hien_o_ten_mien_khach_b`, `token_rut_dong_y_khach_a_vo_hieu_o_khach_b`,
  `nguoi_ban_nhanh_a_khong_xem_vu_viec_nhanh_b` (rò rỉ giữa hai nhánh anh em, bước 34).
- OpenAPI: bài có sẵn kiểm mọi endpoint mới có summary và `description` cho từng response.

## Tiêu chí xong

- [ ] Mọi mục `- [ ]` của bước 37 trong ROADMAP ứng với commit ở bảng "Thứ tự làm" và có test ở trên.
- [ ] `./gate.sh` (đầy đủ) xanh trên SQLite và Postgres; bài sở hữu bảng, migration lên / xuống / lên xanh.
- [ ] `cargo xtask check` xanh: crate chỉ mở `api` và `module`; mọi `code` mới có bản dịch VI; mọi
      `NotificationTemplate` có bản dịch.
- [ ] `docs/openapi.json`, `.env.example` sinh lại; `docs/COMPLIANCE.md` + `.vi.md`,
      `docs/NOTIFICATIONS.md` + `.vi.md`, `docs/MODULE_GUIDE.md` + `.vi.md` (mục `Module::retention`,
      `StartContext::optional`, cách dùng `KycGate` / `PostingGate` / `Notifier`), `docs/COMMENTS.md` +
      `.vi.md` (cách `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES` và `posting.comments` kết hợp, D-202) cùng nội
      dung hai ngôn ngữ.
- [ ] `STACK.md` ghi thư viện mẫu đã chọn và `mrml` (D-206).
- [ ] Mẫu trang EN + VI, mỗi mẫu ghi rõ cần người có chuyên môn pháp lý duyệt.
- [ ] `CHANGELOG.md` có mục; ROADMAP đánh dấu `[x]`; DECISIONS có các mục ở "Agent tự quyết"; phát hành
      theo `docs/RELEASING.md` khi cả bước xong.
- [ ] `autoxide doctor` trên bản cài trống báo website chưa có hồ sơ (warning), không `failed`.

## Chỗ dễ sai

- **NULL trong unique**: SQLite và Postgres đều coi NULL khác nhau — `scope`, `channel`, `org_key` dùng
  `''`. Postgres `NULLS NOT DISTINCT` không có ở SQLite: không dùng.
- **Vòng event**: `compliance.consent_withdrawn` → newsletter huỷ → newsletter gọi `withdraw` → chỉ phát
  event khi trạng thái **đổi**; không đổi thì không phát, nếu không sẽ lặp vô hạn.
- **Ghi ledger trước, module sau**: transaction của newsletter hỏng sau khi ledger đã ghi đồng ý thì không
  sao (người đó thật sự đã đồng ý; newsletter vẫn chưa gửi); ngược lại (module trước, ledger sau) có thể
  gửi quảng cáo không có bằng chứng. Lúc gửi luôn kiểm ledger (đóng khi lỗi: ledger lỗi → không gửi).
- **OTP**: không vào job, log, event, response; so sánh hằng thời gian; dùng một lần bằng UPDATE có điều
  kiện (`rows`), không đọc-rồi-ghi. Trả lời như nhau khi số đã / chưa dùng ở tài khoản khác cho tới bước
  xác nhận (không lộ ai dùng số nào).
- **Gọi mạng trong transaction**: `IdentityVerifier`, `SmsSender`, `ZnsSender`, web push, `Mailer` luôn
  ngoài transaction ghi (CLAUDE.md, luật dữ liệu).
- **Khung giờ**: tính theo giờ địa phương của quy tắc, không theo giờ máy chủ; dùng `utc_offset_minutes`;
  ngày đổi 22:00 → 07:00 hôm sau qua nửa đêm.
- **Ngày làm việc** cho hạn vụ việc: không dùng hàm ngày của database (khác nhau giữa hai bên) — tính
  trong Rust rồi lưu mốc UTC.
- **Ảnh chụp văn bản**: lấy `revision_id` của bản đã đăng, không của bản nháp; SHA-256 tính trên JSON
  chuẩn hoá (khoá sắp xếp) để tính lại được.
- **Snippet logo** là HTML người dùng dán: không bao giờ trả lại nguyên văn — dựng lại từ URL đã kiểm.
- **Event ra ngoài**: `webhooks` chuyển mọi event ra hệ thống ngoài — payload không mang email, số điện
  thoại, số giấy tờ, tên người báo.
- **Thứ tự đăng ký**: `newsletter` phải đứng sau `compliance` / `notifications`; nếu quên, `require` lỗi
  lúc khởi động (tốt) — nhưng bài số lịch của `crates/app` phải sửa. `comments` không phải dời: nó hỏi
  `PostingVerifier` qua chỗ giữ của kernel, không `require` trait của `compliance` (D-202).
- **Bản quy tắc**: chọn bản có `effective_from ≤ t < effective_to` với `version` lớn nhất; hai bản cùng
  ngày hiệu lực → bản `version` lớn thắng. Nạp lại gói đóng sẵn không được ghi đè bản nhập tay.
- **451 và cache**: trang bị ẩn phải mất khỏi cache (`ETag` đổi) và khỏi sitemap / feed.
- **Đếm / chặn theo số điện thoại** dùng `phone_key` (băm), index bắt đầu bằng `tenant_id`.

## Căn cứ pháp lý

Nguồn tổng hợp đối chiếu ngày 13/11/2026 (`business-foundation.md`, mục Nguồn) — **cần đối chiếu văn
bản gốc / người có chuyên môn pháp lý trước khi chạy thật** (D-186). Mọi con số bên dưới nằm trong
`compliance_rules`, không viết cứng.

| Tính năng | Căn cứ | Hiệu lực | Chưa chắc |
|---|---|---|---|
| KYC người bán, người livestream, người tiếp thị liên kết | Luật TMĐT 122/2025/QH15, Điều 17 (người bán); điều về livestream / tiếp thị liên kết | 01/7/2026 | Điều cụ thể cho livestream / tiếp thị; nghị định hướng dẫn (nguồn nêu NĐ 248/2026) |
| Thông báo (logo xanh) / đăng ký (logo đỏ) website; thông tin bắt buộc trên website | NĐ 52/2013 sửa bởi NĐ 85/2021 (mục minh bạch thông tin, thông báo / đăng ký); từ 01/7/2026 theo Luật TMĐT 2025 + nghị định hướng dẫn | — | Danh mục hồ sơ, mẫu tờ khai mới; miền của logo; bán trước khi thông báo website bị **cấm** hay chỉ **bị phạt** (quyết cổng chặn hay nhãn, A-3.5) |
| Xác thực tài khoản đăng nội dung bằng số di động tại Việt Nam / định danh điện tử | NĐ 147/2024/NĐ-CP | 25/12/2024 | Phạm vi áp cho bình luận / đánh giá ngoài mạng xã hội; số nước ngoài |
| Đồng ý, căn cứ xử lý, quyền chủ thể, báo vi phạm 72 giờ, dữ liệu nhạy cảm (sinh trắc học) | Luật BVDLCN 91/2025/QH15; NĐ 356/2025/NĐ-CP | 01/01/2026 | Cơ quan nhận báo cáo và mẫu; mẫu hồ sơ đánh giá tác động; hạn trả lời yêu cầu của chủ thể (D-217); căn cứ xử lý khi khách bật đo mở thư (D-206) |
| Lưu dữ liệu tại Việt Nam | Luật An ninh mạng 116/2025/QH15; NĐ 53/2022/NĐ-CP | 01/7/2026 (luật) | Đối tượng bắt buộc lưu tại Việt Nam |
| Văn bản có phiên bản, bằng chứng chấp nhận | Luật Giao dịch điện tử 20/2023/QH15 | 01/7/2024 | Yêu cầu lưu trữ thông điệp dữ liệu |
| Điều kiện giao dịch chung, khiếu nại, không chặn / ẩn đánh giá trái quy định | Luật BVQLNTD 19/2023/QH15; NĐ 55/2024/NĐ-CP | 01/7/2024 | Thời hạn phản hồi khiếu nại (mặc định dữ liệu: 7 ngày làm việc); danh sách lý do được ẩn |
| Gỡ hàng / nội dung vi phạm SHTT, hàng giả; cung cấp thông tin cho cơ quan | Luật TMĐT 2025; Luật SHTT; NĐ 85/2021 (thời hạn gỡ khi cơ quan yêu cầu) | — | Thời hạn gỡ (mặc định dữ liệu: 24 giờ với yêu cầu của cơ quan) |
| Nhãn nội dung do AI tạo / sửa | Luật Trí tuệ nhân tạo 134/2025/QH15 và nghị định hướng dẫn | 01/3/2026 | Hình thức nhãn; phạm vi "sửa" |
| Tin quảng cáo: đồng ý trước, từ chối dừng ngay, 7h–22h, một tin mời đăng ký, nhãn "QC" | NĐ 91/2020/NĐ-CP (Điều 13 và lân cận) | 08/10/2020 | Bản thay thế đang dự thảo; giới hạn số tin / ngày; danh sách không quảng cáo |
| Zalo ZNS | Chính sách của nhà cung cấp (không phải luật) | — | Loại mẫu được phép cho quảng cáo |

## Agent làm bước này tự quyết

Ghi `docs/DECISIONS.md` (skill `record-decision`) cho mỗi mục đã chốt; mục pháp lý ghi căn cứ.

1. **Trang bắt buộc ở `content`, ảnh chụp ở `compliance`** — khuyến nghị như trên (một cách soạn trang;
   bằng chứng chấp nhận tự đủ). Ghi quyết định vì nó thêm `pub mod api` cho `content` và đặt thứ tự
   `content → compliance`.
2. **Dời `newsletter` sau `compliance` / `notifications`** và `compliance` không dùng `Notifier` — ghi quyết
   định (đổi thứ tự đăng ký có sẵn). `comments` không phải dời (D-202).
3. **`ZnsSender` ở `notifications::api` hay kernel** — khuyến nghị `notifications::api` (chỉ thông báo dùng).
4. **Web push**: tự viết RFC 8291 / 8292 bằng `p256` (đã có, bật `ecdh`), `hkdf`, `aes-gcm` hay dùng crate
   có sẵn — tra bằng `check-library-api`, đo số dòng; thêm thư viện thì ghi `STACK.md`.
5. **Mặc định `posting.*`**: khuyến nghị `none` (blog, website bán hàng không phải mạng xã hội) và tài liệu
   nói rõ khi nào nên bật — cần đối chiếu NĐ 147.
6. **Ngày lễ** trong `cases.deadlines`: danh sách theo năm trong dữ liệu; Tết âm lịch nhập tay mỗi năm.
7. **Danh sách "không quảng cáo" quốc gia (NĐ 91/2020)**: chưa có API công khai đã biết — khuyến nghị
   để trait `DoNotContactList` cho bước sau, ghi ở "Sau core".
8. **Gói quy tắc `vn` ban đầu**: nội dung danh sách kiểm, hạn, lý do ẩn — viết từ nguồn ở `business-foundation.md`,
   mọi mục `needs_legal_review: true`.
9. **Thư viện ngôn ngữ mẫu** (D-206): `liquid` (cú pháp Liquid — Shopify dùng cho mẫu thông báo; chủ cửa
   hàng quen) hay `minijinja` (có vùng cách ly và giới hạn chạy). Đo (thời gian dựng 1 000 mẫu, kích thước
   binary), tra API bằng skill `check-library-api`, chọn một, ghi `STACK.md` và DECISIONS.
10. **"Chặn bán" khi website chưa thông báo** (A-3.5): đối chiếu văn bản (A3) rồi chọn cổng chặn
    `compliance.website_not_notified` hay nhãn "bắt buộc theo luật"; ghi kết quả đối chiếu vào DECISIONS
    trước khi chọn.
11. **Permission cho endpoint đặc tả chưa nêu**: `POST /kyc/profiles/{id}/request-info`,
    `GET /documents/{id}/acceptance-stats`, `GET/POST /subject-requests`, `POST /subject-requests/{id}/fulfil`
    — chọn trong permission của module và ghi DECISIONS.
