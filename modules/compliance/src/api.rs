//! Hợp đồng của module `compliance` cho module khác: cổng KYC, cổng chấp nhận văn bản pháp lý, sổ
//! đồng ý, quy tắc pháp lý có ngày hiệu lực. Module khác lấy lúc khởi động:
//! `context.require::<dyn KycGate>()`… (đăng ký SAU `compliance`).
//!
//! Căn cứ (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý): Luật Thương mại điện tử
//! 122/2025/QH15 Điều 17 (định danh người bán, người livestream, tiếp thị liên kết, từ 01/7/2026); Luật
//! Giao dịch điện tử 20/2023/QH15 (chấp nhận điều khoản điện tử); Luật Bảo vệ dữ liệu cá nhân
//! 91/2025/QH15 + NĐ 356/2025 (đồng ý theo mục đích, rút lại).

use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{OrgId, TenantId};
use chrono::{DateTime, Utc};
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
    pub verified_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
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
    pub expires_at: Option<DateTime<Utc>>,
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
    pub granted_at: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
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
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
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
        at: DateTime<Utc>,
    ) -> Result<Option<Rule>, Error>;
}

/// Event phát khi hồ sơ định danh đổi trạng thái: payload có `profile_id`, `subject_kind`,
/// `subject_id`, `status`. Module gọi (vd `plans` với `Subject::Reference`) nghe để đi tiếp.
pub const KYC_VERIFIED: &str = "compliance.kyc_verified";
pub const KYC_REJECTED: &str = "compliance.kyc_rejected";
