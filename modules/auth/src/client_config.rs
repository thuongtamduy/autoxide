//! Phần của `auth` trong cấu hình cho giao diện (`GET /api/v1/kernel/client-config`): cách đăng nhập
//! nào máy chủ cho dùng, để trang đăng nhập chỉ hiện nút dùng được. Không có secret nào.

use autoxide_kernel::error::Error;
use autoxide_kernel::module::ClientSection;
use serde::Serialize;
use utoipa::ToSchema;

use crate::config::AuthConfig;

/// Sign-in settings an interface needs.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = AuthClientConfig)]
struct AuthClientConfig {
    /// `true` when people can sign in with a link sent by email (`AUTOXIDE_AUTH_LOGIN_LINK`,
    /// `POST /api/v1/auth/login/link`): show "sign in with a link".
    login_link: bool,
    /// `true` when passkeys are set up for this tenant (its setting `auth.passkey_rp_id`, otherwise
    /// `AUTOXIDE_AUTH_PASSKEY_RP_ID`): show "sign in with a passkey".
    passkeys: bool,
    /// The domain passkeys belong to for this tenant (the WebAuthn relying party id); empty when
    /// passkeys are off. A page on another domain cannot use them.
    #[schema(example = "shop.example")]
    passkey_rp_id: String,
    /// `true` when people can create their own staff account in this tenant (its setting
    /// `auth.staff_registration`, otherwise `AUTOXIDE_AUTH_SELF_REGISTRATION`): show "create an
    /// account".
    self_registration: bool,
    /// Longest support access to a tenant that platform staff can be granted, in hours
    /// (`AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS`; `hours` of `POST /api/v1/auth/support-grants`).
    #[schema(example = 72)]
    support_access_max_hours: u32,
}

pub(crate) fn section(config: &AuthConfig) -> Result<Option<ClientSection>, Error> {
    let section = AuthClientConfig {
        login_link: config.login_link,
        passkeys: config.passkey.is_some(),
        passkey_rp_id: config
            .passkey
            .as_ref()
            .map(|passkey| passkey.rp_id.clone())
            .unwrap_or_default(),
        self_registration: config.self_registration,
        support_access_max_hours: config.support_max_hours,
    };
    ClientSection::new(&section).map(Some)
}

/// Cách đăng nhập theo khách: passkey (`auth.passkey_rp_id`) và tự đăng ký nhân sự (`auth.staff_registration`).
pub(crate) struct TenantSignIn {
    pub passkeys: std::sync::Arc<crate::passkey::PasskeyService>,
    pub orgs: Option<std::sync::Arc<dyn autoxide_organizations::api::OrgDirectory>>,
    pub self_registration: bool,
}

#[async_trait::async_trait]
impl autoxide_kernel::module::TenantClientSection for TenantSignIn {
    async fn overrides(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
    ) -> Result<serde_json::Map<String, serde_json::Value>, Error> {
        let passkey = self.passkeys.config_for(tenant).await?;
        let open = crate::settings::enabled_or(
            self.orgs.as_ref(),
            tenant,
            &crate::settings::STAFF_REGISTRATION,
            self.self_registration,
        )
        .await?;
        let mut fields = serde_json::Map::new();
        fields.insert("passkeys".to_owned(), serde_json::json!(passkey.is_some()));
        let rp_id = passkey.map(|passkey| passkey.rp_id).unwrap_or_default();
        fields.insert("passkey_rp_id".to_owned(), serde_json::json!(rp_id));
        fields.insert("self_registration".to_owned(), serde_json::json!(open));
        Ok(fields)
    }
}
