//! Bộ xác thực module `auth` công bố cho cả chương trình (`dyn Authenticator`):
//! access token → người dùng → permission của các role của họ.

use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::error::Error;

use crate::role::RoleService;
use crate::session::SessionService;

pub(crate) struct RequestAuthenticator {
    pub sessions: Arc<SessionService>,
    pub roles: Arc<RoleService>,
}

#[async_trait::async_trait]
impl Authenticator for RequestAuthenticator {
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
        let user = self.sessions.authenticate(authorization).await?;
        self.roles.principal(user.id).await
    }
}
