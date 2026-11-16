//! Bộ xác thực module `auth` công bố cho cả chương trình (`dyn Authenticator`).
//!
//! Header `Authorization: Bearer <token>` mang một trong hai thứ:
//!
//! - **API key** (bắt đầu bằng `axk1_`, key cũ `axk_`): quyền = permission của key ∩ quyền hiện
//!   tại của chủ key;
//! - **access token** của phiên đăng nhập: quyền = permission các role của người dùng.
//!
//! **Khách lấy từ chính credential (bước 33, D-285):** access token mang claim `tid`, API key mang
//! ID khách trong chuỗi (`axk1_<ID khách>_…`); credential cấp trước bước 33 (không mang khách)
//! thuộc khách mặc định. Bộ xác thực tìm khách đó qua sổ database rồi mới tra người dùng / bản băm
//! của key ở database của khách — không cần header `Autoxide-Tenant` hay tên miền. Kernel so khách
//! của credential với header / tên miền của request (`tenant_mismatch`, nhân sự nền tảng).

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, TenantContext, TenantHint, TenantId, ensure_serving,
};

use crate::api_key::{self, ApiKeyService};
use crate::role::RoleService;
use crate::session::{SessionService, bearer_token};

pub(crate) struct RequestAuthenticator {
    pub sessions: Arc<SessionService>,
    pub roles: Arc<RoleService>,
    pub api_keys: Arc<ApiKeyService>,
    /// Sổ database: tìm khách của credential.
    pub databases: DatabaseRegistry,
    /// Uỷ quyền theo cây khách hàng: vào khách con (D-310).
    pub delegations: Arc<crate::delegation::DelegationService>,
}

#[async_trait::async_trait]
impl Authenticator for RequestAuthenticator {
    // `_hint` (tên miền, header) không dùng để định tuyến: khách lấy từ credential; kernel tự so
    // nó với khách của credential sau bước này.
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _hint: &TenantHint,
    ) -> Result<Principal, Error> {
        let token = bearer_token(authorization)?;
        if let Some(tenant) = api_key::tenant_of(token)? {
            let tenant = self.tenant(tenant, api_key::invalid_api_key).await?;
            return self.api_keys.authenticate(&tenant, token).await;
        }
        // Chữ ký kiểm bằng khoá của khách ghi trong token TRƯỚC khi tra khách: `tid` giả không
        // dò được khách nào tồn tại.
        let (tenant, user_id) = self.sessions.verify_access(token)?;
        let tenant = self.tenant(tenant, crate::token::invalid_token).await?;
        let user = self.sessions.token_user(&tenant, user_id).await?;
        // Người mua: chỉ là tài khoản của khách, không permission nào (D-309).
        if user.realm == autoxide_kernel::access::Realm::Customer {
            return Ok(Principal::customer(user.id).in_tenant(tenant.id()));
        }
        self.roles.principal(&tenant, user.id).await
    }

    async fn enter(
        &self,
        principal: &Principal,
        target: &TenantContext,
    ) -> Result<Option<Principal>, Error> {
        self.delegations.enter(principal, target).await
    }
}

impl RequestAuthenticator {
    /// Khách `id` ghi trong credential. Không có → `invalid` (như credential sai: không lộ khách
    /// nào tồn tại). Tạm khoá / bảo trì → `tenant_suspended` / `tenant_maintenance` như mọi
    /// request tới khách đó: client giữ credential và thử lại, thay vì tưởng credential hỏng.
    async fn tenant(&self, id: TenantId, invalid: fn() -> Error) -> Result<TenantContext, Error> {
        let tenant = match self.databases.tenant(id).await {
            Ok(tenant) => tenant,
            Err(error) if error.kind() == ErrorKind::NotFound => return Err(invalid()),
            Err(error) => return Err(error),
        };
        ensure_serving(&tenant)?;
        Ok(tenant)
    }
}
