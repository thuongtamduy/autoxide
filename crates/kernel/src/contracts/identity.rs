//! Xác thực danh tính (KYC) qua nhà cung cấp (bước 37, D-186): trait trung lập để module
//! `compliance` dùng VNeID (qua nhà cung cấp được phép), eKYC hay tra cứu đăng ký kinh doanh mà không
//! phụ thuộc nhà cung cấp nào. Bộ nối của một nhà cung cấp là module riêng, công bố
//! `context.provide::<dyn IdentityVerifier>(…)`. Test dùng `FakeIdentityVerifier` của testkit.
//!
//! Lưu tối thiểu (Luật Bảo vệ dữ liệu cá nhân 91/2025): `compliance` chỉ lưu kết quả, mã tham chiếu, tên
//! đã xác thực, băm và 4 số cuối của số giấy tờ — không lưu ảnh khuôn mặt hay dữ liệu sinh trắc học.
use crate::time::Timestamp;
use uuid::Uuid;

use crate::error::{Error, ErrorKind};
use crate::tenancy::TenantId;

/// Loại bên được xác thực.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartyType {
    Individual,
    HouseholdBusiness,
    Organization,
}

impl PartyType {
    pub fn as_str(self) -> &'static str {
        match self {
            PartyType::Individual => "individual",
            PartyType::HouseholdBusiness => "household_business",
            PartyType::Organization => "organization",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "individual" => Some(PartyType::Individual),
            "household_business" => Some(PartyType::HouseholdBusiness),
            "organization" => Some(PartyType::Organization),
            _ => None,
        }
    }
}

/// Cách xác thực.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerificationMethod {
    /// Tài khoản định danh điện tử VNeID (qua nhà cung cấp được phép).
    Vneid,
    /// eKYC: ảnh giấy tờ + khuôn mặt, nhà cung cấp đối chiếu.
    Ekyc,
    /// Tra cứu đăng ký kinh doanh.
    BusinessRegistry,
    /// Người duyệt xem giấy tờ tải lên.
    Manual,
}

impl VerificationMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            VerificationMethod::Vneid => "vneid",
            VerificationMethod::Ekyc => "ekyc",
            VerificationMethod::BusinessRegistry => "business_registry",
            VerificationMethod::Manual => "manual",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "vneid" => Some(VerificationMethod::Vneid),
            "ekyc" => Some(VerificationMethod::Ekyc),
            "business_registry" => Some(VerificationMethod::BusinessRegistry),
            "manual" => Some(VerificationMethod::Manual),
            _ => None,
        }
    }
}

/// Một lượt xác thực cần bắt đầu.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerificationRequest {
    pub tenant: TenantId,
    /// ID lượt thử trong hệ thống (nhà cung cấp gửi lại trong callback).
    pub attempt_id: Uuid,
    pub party: PartyType,
    pub method: VerificationMethod,
    pub return_url: Option<String>,
    pub locale: String,
}

impl VerificationRequest {
    pub fn new(
        tenant: TenantId,
        attempt_id: Uuid,
        party: PartyType,
        method: VerificationMethod,
    ) -> Self {
        VerificationRequest {
            tenant,
            attempt_id,
            party,
            method,
            return_url: None,
            locale: "vi".to_owned(),
        }
    }

    #[must_use]
    pub fn return_url(mut self, url: Option<String>) -> Self {
        self.return_url = url;
        self
    }

    #[must_use]
    pub fn locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = locale.into();
        self
    }
}

/// Cách người dùng tiếp tục lượt xác thực.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationSession {
    /// Mở trang của nhà cung cấp.
    Redirect { provider_reference: String, url: String, expires_at: Timestamp },
    /// Ứng dụng di động mở SDK bằng token.
    Sdk { provider_reference: String, token: String, expires_at: Timestamp },
    /// Không cần người dùng làm gì (vd tra cứu đăng ký kinh doanh); kết quả về sau.
    Pending { provider_reference: String },
}

impl VerificationSession {
    /// Mã lượt xác thực ở nhà cung cấp.
    pub fn provider_reference(&self) -> &str {
        match self {
            VerificationSession::Redirect { provider_reference, .. }
            | VerificationSession::Sdk { provider_reference, .. }
            | VerificationSession::Pending { provider_reference } => provider_reference,
        }
    }
}

/// Kết quả xác thực.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutcomeStatus {
    Verified,
    Rejected { reason_code: String },
    Pending,
}

/// Kết quả nhà cung cấp trả.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerificationOutcome {
    pub provider_reference: String,
    pub status: OutcomeStatus,
    pub verified_name: Option<String>,
    /// Loại giấy tờ (`cccd`, `passport`, `business_registration`…).
    pub document_kind: Option<String>,
    /// Số giấy tờ: `compliance` chỉ lưu băm và 4 số cuối, không lưu nguyên.
    pub document_number: Option<String>,
    /// Mức bảo đảm, vd `vneid_level_2`.
    pub assurance: Option<String>,
    pub expires_at: Option<Timestamp>,
}

impl VerificationOutcome {
    pub fn new(provider_reference: impl Into<String>, status: OutcomeStatus) -> Self {
        VerificationOutcome {
            provider_reference: provider_reference.into(),
            status,
            verified_name: None,
            document_kind: None,
            document_number: None,
            assurance: None,
            expires_at: None,
        }
    }

    #[must_use]
    pub fn verified_name(mut self, name: impl Into<String>) -> Self {
        self.verified_name = Some(name.into());
        self
    }

    #[must_use]
    pub fn document(mut self, kind: impl Into<String>, number: impl Into<String>) -> Self {
        self.document_kind = Some(kind.into());
        self.document_number = Some(number.into());
        self
    }

    #[must_use]
    pub fn assurance(mut self, assurance: impl Into<String>) -> Self {
        self.assurance = Some(assurance.into());
        self
    }
}

/// Một nhà cung cấp xác thực danh tính.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait IdentityVerifier: Send + Sync {
    /// Tên ổn định (`vnpt_ekyc`, `fake`…): nằm trong đường dẫn callback.
    fn provider(&self) -> &'static str;

    fn methods(&self) -> Vec<VerificationMethod>;

    async fn start(&self, request: VerificationRequest) -> Result<VerificationSession, Error>;

    async fn fetch(&self, provider_reference: &str) -> Result<VerificationOutcome, Error>;

    /// Kiểm chữ ký callback trên thân thô; sai → [`callback_invalid`].
    async fn verify_callback(
        &self,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> Result<VerificationOutcome, Error>;
}

/// `identity_verification_disabled` (503): chưa cấu hình nhà cung cấp.
pub fn verification_disabled() -> Error {
    Error::unavailable(
        "identity_verification_disabled",
        "No identity verification provider is set up on this server.",
    )
}

/// `identity_callback_invalid` (401): chữ ký callback sai.
pub fn callback_invalid() -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        "identity_callback_invalid",
        "The signature of the identity provider callback is not valid.",
    )
}
