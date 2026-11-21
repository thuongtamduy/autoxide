//! Hợp đồng của module `compliance` cho module khác: cổng KYC, cổng chấp nhận văn bản pháp lý, sổ
//! đồng ý, quy tắc pháp lý có ngày hiệu lực. Module khác lấy lúc khởi động:
//! `context.require::<dyn KycGate>()`… (đăng ký SAU `compliance`).
//!
//! Căn cứ (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý): Luật Thương mại điện tử
//! 122/2025/QH15 Điều 17 (định danh người bán, người livestream, tiếp thị liên kết, từ 01/7/2026); Luật
//! Giao dịch điện tử 20/2023/QH15 (chấp nhận điều khoản điện tử); Luật Bảo vệ dữ liệu cá nhân
//! 91/2025/QH15 + NĐ 356/2025 (đồng ý theo mục đích, rút lại).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantContext, TenantId};
use autoxide_kernel::time::Timestamp;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Việc cần định danh trước (Luật TMĐT 2025 Điều 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = ComplianceActivity)]
pub enum Activity {
    Sell,
    Livestream,
    Affiliate,
}

impl Activity {
    pub fn as_str(self) -> &'static str {
        match self {
            Activity::Sell => "sell",
            Activity::Livestream => "livestream",
            Activity::Affiliate => "affiliate",
        }
    }
}

/// Đối tượng của hồ sơ định danh hay của lần chấp nhận văn bản.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subject {
    /// Một người dùng của khách.
    User(Uuid),
    /// Một nút tổ chức (người bán, chi nhánh).
    Org(OrgId),
    /// Một email chưa có tài khoản (vd người đăng ký khách mới chấp nhận điều khoản dịch vụ).
    Email(String),
    /// Mã do module gọi đặt khi chưa có đối tượng nào khác (vd `plans.signup:<id>`): KYC trước khi
    /// có khách.
    Reference(String),
}

impl Subject {
    /// (`subject_kind`, `subject_id`) lưu trong bảng.
    pub fn columns(&self) -> (&'static str, String) {
        match self {
            Subject::User(id) => ("user", id.to_string()),
            Subject::Org(org) => ("org", org.as_uuid().to_string()),
            Subject::Email(email) => ("email", email.trim().to_lowercase()),
            Subject::Reference(reference) => ("reference", reference.clone()),
        }
    }
}

/// KYC status of a subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceKycStatus)]
pub struct KycStatus {
    /// `none` (no profile), `draft`, `pending`, `needs_info`, `verified`, `rejected`, `expired` or
    /// `revoked`.
    pub status: String,
    pub profile_id: Option<Uuid>,
    pub verified_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
}

/// Cổng định danh.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait KycGate: Send + Sync {
    /// Ok khi quy tắc đang hiệu lực không đòi định danh cho `activity`, hoặc đối tượng có hồ sơ
    /// `verified` còn hạn. Ngược lại `compliance.kyc_required` (403).
    async fn require_verified(
        &self,
        tenant: TenantId,
        subject: Subject,
        activity: Activity,
    ) -> Result<(), Error>;

    async fn status(&self, tenant: TenantId, subject: Subject) -> Result<KycStatus, Error>;

    /// Mở (hay trả lại) hồ sơ định danh của `subject` và bắt đầu một lượt xác thực ở nhà cung cấp;
    /// trả trang / token cho người dùng. Dùng khi chưa có tài khoản (vd đăng ký khách mới).
    async fn start(
        &self,
        tenant: TenantId,
        subject: Subject,
        party: autoxide_kernel::identity::PartyType,
        return_url: Option<String>,
    ) -> Result<KycSession, Error>;
}

/// A started verification: where the person goes next.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceKycSession)]
pub struct KycSession {
    pub profile_id: Uuid,
    pub attempt_id: Uuid,
    /// `redirect` (open `url`), `sdk` (start the mobile SDK with `token`) or `pending` (nothing to
    /// do; the result comes later).
    pub kind: String,
    pub url: Option<String>,
    pub token: Option<String>,
    pub expires_at: Option<Timestamp>,
}

/// How far an account is verified for posting content; `identity` includes `phone`.
//
// Mức xác thực của một tài khoản (NĐ 147/2024/NĐ-CP — cần đối chiếu văn bản gốc). `Identity` bao
// `Phone`: thứ tự khai báo là thứ tự mạnh dần, so sánh bằng `>=`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[schema(as = ComplianceVerificationLevel)]
pub enum VerificationLevel {
    /// Not verified.
    None,
    /// A Vietnamese mobile number was confirmed with a one-time code.
    Phone,
    /// The identity was verified (KYC profile `verified` and not expired).
    Identity,
}

impl VerificationLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            VerificationLevel::None => "none",
            VerificationLevel::Phone => "phone",
            VerificationLevel::Identity => "identity",
        }
    }

    /// Đọc từ giá trị chính sách `posting.<chỗ đăng>` (`none` / `phone` / `identity`).
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(VerificationLevel::None),
            "phone" => Some(VerificationLevel::Phone),
            "identity" => Some(VerificationLevel::Identity),
            _ => None,
        }
    }
}

/// Chỗ người dùng đăng nội dung; khách đặt mức cần cho từng chỗ qua chính sách `posting.<chỗ>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PostingSurface {
    Comments,
    Reviews,
    Livestream,
}

impl PostingSurface {
    /// Khoá chính sách của chỗ đăng, vd `posting.comments`.
    pub fn policy_key(self) -> &'static str {
        match self {
            PostingSurface::Comments => "posting.comments",
            PostingSurface::Reviews => "posting.reviews",
            PostingSurface::Livestream => "posting.livestream",
        }
    }
}

/// Cổng đăng nội dung: so mức khách đặt cho chỗ đăng với mức của tài khoản.
#[async_trait::async_trait]
pub trait PostingGate: Send + Sync {
    /// `None` principal = khách vãng lai, chỉ qua được khi chính sách là `none`. Thiếu mức khách đặt →
    /// `compliance.verification_required` (403, kèm `level`).
    async fn check(
        &self,
        tenant: TenantId,
        principal: Option<&autoxide_kernel::access::Principal>,
        surface: PostingSurface,
    ) -> Result<(), Error>;
}

/// Trạng thái xác thực tài khoản.
#[async_trait::async_trait]
pub trait AccountVerification: Send + Sync {
    async fn level(&self, tenant: TenantId, user_id: Uuid) -> Result<VerificationLevel, Error>;

    /// Số đã xác thực (vd cho `notifications` gửi SMS / ZNS). Chưa có → `None`.
    async fn verified_phone(
        &self,
        tenant: TenantId,
        user_id: Uuid,
    ) -> Result<Option<autoxide_kernel::sms::PhoneNumber>, Error>;
}

/// Loại văn bản pháp lý.
pub const DOCUMENT_KINDS: [&str; 6] = [
    "terms_of_use",
    "privacy_policy",
    "general_conditions",
    "platform_rules",
    "seller_agreement",
    "developer_agreement",
];

/// Cổng chấp nhận văn bản pháp lý.
#[async_trait::async_trait]
pub trait AcceptanceGate: Send + Sync {
    /// Đối tượng đã chấp nhận bản hiện hành cần chấp nhận của văn bản `kind` (cấp khách); không →
    /// `compliance.acceptance_required` (403). Khách chưa có văn bản loại này: Ok.
    async fn require_accepted(
        &self,
        tenant: TenantId,
        subject: Subject,
        kind: &str,
    ) -> Result<(), Error>;

    /// Ghi chấp nhận bản hiện hành của văn bản `kind` cho `subject` (gọi lại vô hại). Trả ID phiên
    /// bản đã chấp nhận; khách chưa có văn bản loại này: `None`.
    async fn accept_current(
        &self,
        tenant: TenantId,
        subject: Subject,
        kind: &str,
        channel: &str,
    ) -> Result<Option<Uuid>, Error>;
}

/// Ai đồng ý.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConsentSubject {
    User(Uuid),
    Email(String),
    /// Số E.164.
    Phone(String),
}

/// Đồng ý cho việc gì: mục đích, phạm vi (`""` = mọi), kênh (`""` = mọi).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsentKey)]
pub struct ConsentKey {
    /// e.g. `marketing`.
    pub purpose: String,
    /// `""` (everything) or e.g. `newsletter.list:<id>`.
    #[serde(default)]
    pub scope: String,
    /// `""` (every channel), `email`, `sms`, `zns`, `push` or `in_app`.
    #[serde(default)]
    pub channel: String,
}

impl ConsentKey {
    pub fn new(
        purpose: impl Into<String>,
        scope: impl Into<String>,
        channel: impl Into<String>,
    ) -> Self {
        ConsentKey { purpose: purpose.into(), scope: scope.into(), channel: channel.into() }
    }
}

/// Bằng chứng đồng ý: câu chữ người dùng thấy, phiên bản câu chữ, nơi đồng ý.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsentEvidence)]
pub struct ConsentEvidence {
    pub text: String,
    pub text_version: String,
    /// Where: `web_form`, `checkout`, `import`…
    pub source: String,
    pub ip_fingerprint: Option<String>,
}

/// A consent, as it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceConsent)]
pub struct Consent {
    pub id: Uuid,
    pub purpose: String,
    pub scope: String,
    pub channel: String,
    /// `granted` or `withdrawn`.
    pub state: String,
    pub granted_at: Option<Timestamp>,
    pub withdrawn_at: Option<Timestamp>,
    /// The email or phone number (`null` for a user, or after erasure).
    pub subject_value: Option<String>,
}

/// Sổ đồng ý theo mục đích.
#[async_trait::async_trait]
pub trait ConsentLedger: Send + Sync {
    /// Ghi đồng ý; gọi lại cùng `source_ref` → không ghi thêm, trả trạng thái hiện tại.
    async fn grant(
        &self,
        tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        evidence: ConsentEvidence,
        source_ref: &str,
    ) -> Result<Consent, Error>;

    async fn withdraw(
        &self,
        tenant: TenantId,
        subject: ConsentSubject,
        key: ConsentKey,
        source: &str,
        source_ref: &str,
    ) -> Result<Consent, Error>;

    /// Có đồng ý còn hiệu lực ở đúng khoá hay khoá rộng hơn (`scope` / `channel` rỗng bao các con), và
    /// không có rút lại ở khoá rộng hơn.
    async fn is_granted(
        &self,
        tenant: TenantId,
        subject: &ConsentSubject,
        key: &ConsentKey,
    ) -> Result<bool, Error>;

    async fn granted_many(
        &self,
        tenant: TenantId,
        subjects: &[ConsentSubject],
        key: &ConsentKey,
    ) -> Result<Vec<bool>, Error>;

    async fn get(&self, tenant: TenantId, consent_id: Uuid) -> Result<Option<Consent>, Error>;
}

/// A legal rule in force.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
#[schema(as = ComplianceRule)]
pub struct Rule {
    pub jurisdiction: String,
    pub key: String,
    pub version: i64,
    pub effective_from: Timestamp,
    pub effective_to: Option<Timestamp>,
    #[schema(value_type = Object)]
    pub body: serde_json::Value,
    pub legal_basis: String,
    /// Check against the original text before relying on it.
    pub needs_legal_review: bool,
}

/// Quy tắc pháp lý có ngày hiệu lực.
#[async_trait::async_trait]
pub trait ComplianceRules: Send + Sync {
    /// Bản quy tắc `key` hiệu lực lúc `at` cho vùng pháp lý của khách (mặc định `vn`).
    async fn effective(
        &self,
        tenant: TenantId,
        key: &str,
        at: Timestamp,
    ) -> Result<Option<Rule>, Error>;
}

/// Event phát khi hồ sơ định danh đổi trạng thái: payload có `profile_id`, `subject_kind`,
/// `subject_id`, `status`. Module gọi (vd `plans` với `Subject::Reference`) nghe để đi tiếp.
pub const KYC_VERIFIED: &str = "compliance.kyc_verified";
pub const KYC_REJECTED: &str = "compliance.kyc_rejected";

/// Event phát khi một đồng ý bị rút: payload có `consent_id`, `purpose`, `scope`, `channel`. Module
/// gửi tin quảng cáo (vd `notifications`) nghe để huỷ tin đang chờ.
pub const CONSENT_WITHDRAWN: &str = "compliance.consent_withdrawn";

/// Event phát khi người dùng xác thực xong số điện thoại (payload `user_id`, không có số) hay gỡ số
/// đã xác thực.
pub const PHONE_VERIFIED: &str = "compliance.phone_verified";
pub const PHONE_REMOVED: &str = "compliance.phone_removed";

/// Loại trang bắt buộc gắn được với một website (danh sách kiểm Bộ Công Thương, mẫu trang của
/// `content`). Cùng khoá với mẫu trang `modules/content/templates/legal/<khoá>.<ngôn ngữ>.json`.
pub const PAGE_KINDS: [&str; 11] = [
    "owner_info",
    "privacy_policy",
    "return_policy",
    "shipping_policy",
    "payment_policy",
    "inspection_policy",
    "complaint_policy",
    "terms_of_use",
    "general_conditions",
    "platform_rules",
    "seller_agreement",
];

/// Website mà một "fact" của danh sách kiểm được hỏi.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct WebsiteRef {
    pub id: Uuid,
    /// Nút của website (nút gốc = cả khách).
    pub org: OrgId,
    pub domain: String,
    /// `sales` hay `platform`.
    pub kind: String,
}

/// Một điều kiện của danh sách kiểm do module khác trả lời (vd bước 46: "trang sản phẩm có thông
/// tin người bán"). `true` = đạt.
#[async_trait::async_trait]
pub trait Fact: Send + Sync {
    async fn holds(&self, tenant: TenantId, website: &WebsiteRef) -> Result<bool, Error>;
}

/// Nơi module đăng ký sau `compliance` góp "fact" cho danh sách kiểm Bộ Công Thương (mục `check.type
/// = "fact"` của quy tắc `moit.checklist.*`). Chưa ai đăng ký khoá đó → mục báo `unknown`.
pub trait ComplianceFacts: Send + Sync {
    /// Đăng ký `fact` cho khoá `key` (vd `marketplace.seller_info_on_product`); khoá đã có → lỗi.
    fn register(&self, key: &'static str, fact: std::sync::Arc<dyn Fact>) -> Result<(), Error>;
}

/// Hồ sơ pháp lý (chủ sở hữu website) cho module khác dùng làm biến mẫu — vd `content` điền mẫu
/// trang bắt buộc. Không trả gì ngoài thông tin vốn phải công khai trên website.
#[async_trait::async_trait]
pub trait LegalProfiles: Send + Sync {
    /// Biến mẫu (`owner.*`, `contact.*`) của hồ sơ áp cho nút `org` — hồ sơ của chính nút, hay của
    /// nút cấp trên gần nhất có hồ sơ (`None`: cả khách). Không có hồ sơ nào → `None`.
    async fn template_variables(
        &self,
        tenant: &TenantContext,
        org: Option<OrgId>,
    ) -> Result<Option<serde_json::Value>, Error>;
}

/// Event phát khi trạng thái thông báo / đăng ký Bộ Công Thương của một website đổi (payload
/// `website_id`, `domain`, `kind`, `status`, `previous_status`).
pub const WEBSITE_STATUS_CHANGED: &str = "compliance.website_status_changed";

/// Luật kiểm duyệt nội dung của người tiêu dùng (bước 37, D-364; Luật Bảo vệ quyền lợi người tiêu
/// dùng 19/2023/QH15, NĐ 55/2024/NĐ-CP — cần đối chiếu văn bản gốc): nền tảng không ẩn đánh giá trái
/// quy định. Module ẩn đánh giá / nội dung của người mua (vd bước 45) hỏi trước khi ẩn và ghi lý do
/// vào event của chính nó. Danh sách lý do là quy tắc `moderation.hide_reasons` (dữ liệu có ngày hiệu
/// lực).
#[async_trait::async_trait]
pub trait ModerationPolicy: Send + Sync {
    /// Các mã lý do được ẩn ở nơi `surface` (vd `reviews`, `comments`).
    async fn hide_reasons(&self, tenant: TenantId, surface: &str) -> Result<Vec<String>, Error>;

    /// `reason` có được phép ở `surface` không; không → `compliance.hide_reason_not_allowed`
    /// (422, kèm `allowed`).
    async fn check_hide_reason(
        &self,
        tenant: TenantId,
        surface: &str,
        reason: &str,
    ) -> Result<(), Error>;
}

/// Khiếu nại người tiêu dùng chuyển từ một phiếu hỗ trợ (bước 53): `compliance` tạo vụ việc
/// `consumer_complaint` có hạn luật định (Luật 19/2023/QH15), đích là phiếu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplaintFromTicket {
    /// Phiếu nguồn, `support.ticket:<id>`: gọi lại cùng giá trị trả vụ việc cũ, không tạo vụ thứ hai.
    pub source_ref: String,
    /// Nút bị khiếu nại (cửa hàng, gian hàng của phiếu).
    pub org_id: Uuid,
    /// Tài khoản người mua, nếu có.
    pub reporter_user_id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    /// Nội dung khiếu nại (10 tới 5000 ký tự).
    pub claim: String,
}

/// Vụ việc nhận khiếu nại.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaseRef {
    pub id: Uuid,
    /// `false`: phiếu đã được chuyển trước đó, đây là vụ việc cũ.
    pub created: bool,
}

/// Nhận khiếu nại từ module khác. Gọi NGOÀI transaction ghi của bên gọi.
#[async_trait::async_trait]
pub trait ComplaintIntake: Send + Sync {
    async fn open(
        &self,
        tenant: &TenantContext,
        complaint: ComplaintFromTicket,
    ) -> Result<CaseRef, Error>;
}
