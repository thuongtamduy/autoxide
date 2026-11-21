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
    /// Quyền vào hỗ trợ của nhân sự nền tảng (D-313).
    pub support: Arc<crate::support::SupportService>,
    /// Rào quyền tối đa khách cấp trên đặt (bước 54): áp cho mọi đường vào.
    pub guardrails: Arc<crate::guardrail::GuardrailService>,
    /// Xác nhận lại cho việc nhạy cảm (bước 54 P5): chỉ phiên đăng nhập.
    pub step_up: crate::step_up::StepUpPolicy,
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
            let principal = self.api_keys.authenticate(&tenant, token).await?;
            return self.guardrails.apply(principal).await;
        }
        // Chữ ký kiểm bằng khoá của khách ghi trong token TRƯỚC khi tra khách: `tid` giả không
        // dò được khách nào tồn tại.
        let verified = self.sessions.verify_access(token)?;
        let (tenant, user_id) = (verified.tenant, verified.user);
        let tenant = self.tenant(tenant, crate::token::invalid_token).await?;
        // Dòng người dùng, quyền và phiên còn sống đọc trong một câu (D-319,
        // D-20261121-access-token-revocation). Token cũ không mang phiên (`sid`) thì chỉ kiểm
        // người dùng, như trước.
        let (user, principal, live) = match verified.session {
            Some(session) => self
                .roles
                .session_principal(&tenant, user_id, session)
                .await?
                .ok_or_else(crate::token::invalid_token)?,
            None => {
                let (user, principal) = self
                    .roles
                    .user_principal(&tenant, user_id)
                    .await?
                    .ok_or_else(crate::token::invalid_token)?;
                (user, principal, true)
            }
        };
        // Tài khoản bị khoá báo `user_disabled` trước: khoá cũng thu hồi mọi phiên, nhưng lý do
        // đúng cho client là "bị khoá", không phải "phiên đã kết thúc".
        if !user.is_active() {
            return Err(crate::user::user_disabled());
        }
        if !live {
            return Err(crate::token::session_ended());
        }
        // Người mua: chỉ là tài khoản của khách, không permission nào (D-309).
        if user.realm == autoxide_kernel::access::Realm::Customer {
            return Ok(Principal::customer(user.id).in_tenant(tenant.id()));
        }
        let principal = self.step_up.apply(&tenant, principal, verified.authenticated_at).await?;
        self.guardrails.apply(principal).await
    }

    async fn enter(
        &self,
        principal: &Principal,
        target: &TenantContext,
    ) -> Result<Option<Principal>, Error> {
        // Người từ khách khác vào cũng bị rào của khách được vào.
        let entered = match self.delegations.enter(principal, target).await? {
            Some(entered) => Some(entered),
            None => self.support.enter(principal, target).await?,
        };
        // Đòi hỏi xác nhận lại của phiên đi theo người vào khách khác.
        let entered = match (entered, principal.step_up()) {
            (Some(entered), Some(step_up)) => Some(entered.with_step_up(step_up)),
            (entered, _) => entered,
        };
        match entered {
            Some(entered) => Ok(Some(self.guardrails.apply(entered).await?)),
            None => Ok(None),
        }
    }

    // Nhân sự nền tảng chỉ vào dữ liệu khách bằng lần cấp có thời hạn (D-190, D-313).
    fn decides_entry(&self) -> bool {
        true
    }

    // Người dùng bị khoá, người mua (không permission) hay không có: `None`.
    async fn principal_of(
        &self,
        tenant: &TenantContext,
        user_id: uuid::Uuid,
    ) -> Result<Option<Principal>, Error> {
        let found = self.roles.user_principal(tenant, user_id).await?;
        let principal = found.and_then(|(user, principal)| {
            (user.is_active() && user.realm != autoxide_kernel::access::Realm::Customer)
                .then_some(principal)
        });
        match principal {
            Some(principal) => Ok(Some(self.guardrails.apply(principal).await?)),
            None => Ok(None),
        }
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
