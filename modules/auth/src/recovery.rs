//! Việc của người vận hành máy chủ, qua CLI (trait [`AccountRecovery`]).

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::TenantContext;

use crate::api::{AccountRecovery, User};
use crate::mfa::MfaService;
use crate::passkey::PasskeyService;
use crate::user::UserService;

pub(crate) struct OperatorRecovery {
    pub users: Arc<UserService>,
    pub mfa: Arc<MfaService>,
    pub passkeys: Arc<PasskeyService>,
}

impl OperatorRecovery {
    async fn account(&self, tenant: &TenantContext, email: &str) -> Result<User, Error> {
        self.users.find_by_email(tenant, email).await?.ok_or_else(|| {
            Error::not_found(
                "auth.account_not_found",
                "There is no account with the email {email}.",
            )
            .with_param("email", email.trim())
        })
    }
}

#[async_trait::async_trait]
impl AccountRecovery for OperatorRecovery {
    async fn reset_mfa(&self, tenant: &TenantContext, email: &str) -> Result<User, Error> {
        let user = self.account(tenant, email).await?;
        // Người vận hành máy chủ, tức "hệ thống"; event ghi `by_administrator`.
        self.mfa.reset(tenant, Actor::system(), user.id).await?;
        tracing::info!(user_id = %user.id, "two-step sign-in turned off by the server operator");
        Ok(user)
    }

    async fn remove_passkeys(
        &self,
        tenant: &TenantContext,
        email: &str,
    ) -> Result<(User, u64), Error> {
        let user = self.account(tenant, email).await?;
        let removed = self.passkeys.remove_all_for(tenant, Actor::system(), user.id).await?;
        Ok((user, removed))
    }
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::time::Timestamp;
    use autoxide_testkit::TestDatabase;

    use super::*;
    use crate::api::{Accounts, NewAccount};

    #[tokio::test]
    async fn nguoi_van_hanh_go_mfa_va_passkey_theo_email() {
        TestDatabase::run(async |test| {
            let services = crate::testing::services(test).await;
            let tenant = services.tenant.clone();
            let account = NewAccount {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: None,
                admin: true,
            };
            let ada =
                Accounts::create(services.users.as_ref(), &services.tenant, account).await.unwrap();
            let setup = services.mfa.start_totp(&tenant, ada.id, "ada@example.com").await.unwrap();
            let now = u64::try_from(Timestamp::now().as_unix_seconds()).unwrap();
            let code = crate::mfa::service::tests::code_at(&setup.secret, now);
            services.mfa.confirm_totp(&tenant, ada.id, &code).await.unwrap();
            assert!(services.mfa.status(&tenant, ada.id).await.unwrap().totp_enabled);

            let recovery = OperatorRecovery {
                users: services.users.clone(),
                mfa: services.mfa.clone(),
                passkeys: services.passkeys.clone(),
            };
            assert_eq!(
                recovery.reset_mfa(&services.tenant, " ADA@example.com ").await.unwrap().id,
                ada.id
            );
            assert!(!services.mfa.status(&tenant, ada.id).await.unwrap().totp_enabled);
            let missing =
                recovery.reset_mfa(&services.tenant, "ghost@example.com").await.unwrap_err();
            assert_eq!(missing.code(), "auth.account_not_found");

            // Gỡ passkey (đặc tả 32 B2).
            let pass = "correct horse battery staple";
            services
                .users
                .set_initial_password(&tenant, Actor::system(), ada.id, pass.to_owned())
                .await
                .unwrap();
            let mut device = crate::passkey::testing::SoftAuthenticator::new(4);
            let ceremony =
                services.passkeys.start_registration(&tenant, ada.id, pass).await.unwrap();
            let credential = device.register(&ceremony.options);
            services
                .passkeys
                .finish_registration(&tenant, ada.id, ceremony.id, "Phone", &credential)
                .await
                .unwrap();
            let (user, removed) =
                recovery.remove_passkeys(&services.tenant, "ada@example.com").await.unwrap();
            assert_eq!((user.id, removed), (ada.id, 1));
            assert_eq!(
                recovery.remove_passkeys(&services.tenant, "ada@example.com").await.unwrap().1,
                0
            );
            let missing =
                recovery.remove_passkeys(&services.tenant, "ghost@example.com").await.unwrap_err();
            assert_eq!(missing.code(), "auth.account_not_found");
        })
        .await
        .unwrap();
    }
}
