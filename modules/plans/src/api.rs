//! Hợp đồng của module `plans` cho module khác và cho binary.
//!
//! Quyền dùng KHÔNG đi qua đây: module khác hỏi `autoxide_kernel::entitlement::EntitlementsHandle`
//! (lấy bằng `StartContext::entitlements`), module `plans` chỉ là bên cung cấp. Ở đây là phần cấp
//! phát khách mới theo gói (bước 35 mục 5), cho lệnh `autoxide tenant create … --plan`.

use autoxide_kernel::error::Error;
use uuid::Uuid;

/// Một khách mới cần cấp phát theo gói.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ProvisionRequest {
    pub slug: String,
    pub name: String,
    pub plan_id: Uuid,
    pub admin_email: String,
    pub price_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    /// Tên placement; `None`: `main` (gói `shared`).
    pub placement: Option<String>,
    pub admin_name: Option<String>,
    /// `None`: theo `sample_data` của gói.
    pub sample_data: Option<bool>,
    /// Cho dùng thử nếu gói có.
    pub trial: bool,
}

impl ProvisionRequest {
    pub fn new(
        slug: impl Into<String>,
        name: impl Into<String>,
        plan_id: Uuid,
        admin_email: impl Into<String>,
    ) -> Self {
        ProvisionRequest {
            slug: slug.into(),
            name: name.into(),
            plan_id,
            admin_email: admin_email.into(),
            price_id: None,
            parent_id: None,
            placement: None,
            admin_name: None,
            sample_data: None,
            trial: true,
        }
    }
}

/// Kết quả một lượt cấp phát.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProvisioningStatus {
    pub id: Uuid,
    pub tenant_id: Option<Uuid>,
    /// Bước đã xong gần nhất (`done` khi xong).
    pub step: String,
    /// `pending`, `running`, `failed`, `done`.
    pub status: String,
}

impl ProvisioningStatus {
    pub fn new(id: Uuid, tenant_id: Option<Uuid>, step: String, status: String) -> Self {
        ProvisioningStatus { id, tenant_id, step, status }
    }
}

/// Cấp phát khách mới theo gói: tạo khách → gán gói → dữ liệu mẫu → mời quản trị viên. Không kiểm
/// permission — bên gọi chịu trách nhiệm (lệnh CLI chạy với quyền của người vận hành).
#[async_trait::async_trait]
pub trait Provisioning: Send + Sync {
    /// Chạy hết các bước ngay (không qua job). Bước hỏng: lỗi `plans.provisioning_failed`; lượt đã
    /// ghi lại, chạy tiếp được bằng `POST /api/v1/plans/provisionings/{id}/retry`.
    async fn provision(&self, request: ProvisionRequest) -> Result<ProvisioningStatus, Error>;
}
