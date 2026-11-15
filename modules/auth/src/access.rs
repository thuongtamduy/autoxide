//! Bộ xác thực module `auth` công bố cho cả chương trình (`dyn Authenticator`).
//!
//! Header `Authorization: Bearer <token>` mang một trong hai thứ:
//!
//! - **API key** (bắt đầu bằng `axk_`): quyền = permission của key ∩ quyền hiện tại
//!   của chủ key;
//! - **access token** của phiên đăng nhập: quyền = permission các role của người dùng.

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{DatabaseRegistry, TenantHint};

use crate::api_key::{API_KEY_PREFIX, ApiKeyService};
use crate::role::RoleService;
use crate::session::{SessionService, bearer_token};

pub(crate) struct RequestAuthenticator {
    pub sessions: Arc<SessionService>,
    pub roles: Arc<RoleService>,
    pub api_keys: Arc<ApiKeyService>,
    /// Sổ database: tìm khách của credential.
    pub databases: DatabaseRegistry,
}

#[async_trait::async_trait]
impl Authenticator for RequestAuthenticator {
    // `hint` chưa dùng: credential chưa mang khách; mọi credential hiện thuộc khách mặc định
    // (principal đặt `in_tenant` theo khách đã tra).
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _hint: &TenantHint,
    ) -> Result<Principal, Error> {
        // B12b: credential mang khách — đọc khách trong access token (`tid`) / API key rồi tra ở
        // database của khách đó, thay cho khách mặc định.
        let tenant = self.databases.default_tenant();
        let token = bearer_token(authorization)?;
        if token.starts_with(API_KEY_PREFIX) {
            return self.api_keys.authenticate(&tenant, token).await;
        }
        let user = self.sessions.authenticate(&tenant, authorization).await?;
        self.roles.principal(&tenant, user.id).await
    }
}
