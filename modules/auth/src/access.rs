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

use crate::api_key::{API_KEY_PREFIX, ApiKeyService};
use crate::role::RoleService;
use crate::session::{SessionService, bearer_token};

pub(crate) struct RequestAuthenticator {
    pub sessions: Arc<SessionService>,
    pub roles: Arc<RoleService>,
    pub api_keys: Arc<ApiKeyService>,
}

#[async_trait::async_trait]
impl Authenticator for RequestAuthenticator {
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
        let token = bearer_token(authorization)?;
        if token.starts_with(API_KEY_PREFIX) {
            return self.api_keys.authenticate(token).await;
        }
        let user = self.sessions.authenticate(authorization).await?;
        self.roles.principal(user.id).await
    }
}
