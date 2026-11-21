//! Gỡ liên kết đăng nhập ngoài khi hai request chạy cùng lúc (mục 24 của bản rà lỗi đua 11/2026,
//! D-181): tài khoản không mật khẩu, không passkey, có đúng hai liên kết; gỡ cả hai cùng lúc thì
//! đúng một lần được, lần kia `auth.identity_last_method` — không bao giờ mất cả hai.

use std::sync::Arc;

use autoxide_kernel::event::{Actor, Outbox};
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::tenancy::DatabaseRegistry;
use autoxide_testkit::TestDatabase;
use uuid::Uuid;

use super::identity::{self, External};
use super::{OidcClient, OidcService, ProviderService};
use crate::testing::Services;
use crate::user::NewUser;

fn service(services: &Services) -> OidcService {
    let secrets = SecretKey::new(services.database.clone(), None);
    OidcService {
        enabled: true,
        providers: ProviderService { secrets: secrets.clone(), outbox: Outbox::standalone("auth") },
        client: OidcClient::new(false).unwrap(),
        users: services.users.clone(),
        sessions: services.sessions.clone(),
        outbox: Outbox::standalone("auth"),
        secrets,
        databases: DatabaseRegistry::single(services.database.clone()),
        orgs: None,
        entitlements: Default::default(),
        self_registration: false,
    }
}

/// Người dùng không mật khẩu với hai liên kết; trả ID người dùng và hai ID liên kết.
async fn two_links(services: &Services, email: &str) -> (Uuid, Uuid, Uuid) {
    let tenant = &services.tenant;
    let input = NewUser { email: email.to_owned(), name: "Grace".to_owned(), password: None };
    let user = services.users.create(tenant, Actor::system(), input).await.unwrap();
    let mut ids = Vec::new();
    for subject in ["a", "b"] {
        let external = External {
            subject: format!("{email}-{subject}"),
            email: Some(email.to_owned()),
            email_verified: true,
            name: None,
        };
        let write = tenant.database().write();
        let linked = identity::insert(write, tenant, user.id, Uuid::now_v7(), &external).await;
        ids.push(linked.unwrap().id);
    }
    (user.id, ids[0], ids[1])
}

#[tokio::test]
async fn go_hai_lien_ket_cung_luc_khong_mat_ca_hai() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let oidc = Arc::new(service(&services));
        let tenant = services.tenant.clone();
        // Lặp vài lần: thứ tự chen giữa trên Postgres không cố định.
        for round in 0..3 {
            let (user, first, second) =
                two_links(&services, &format!("grace{round}@example.com")).await;
            let (left, right) = tokio::join!(
                oidc.unlink(&tenant, user, first),
                oidc.unlink(&tenant, user, second),
            );
            let codes: Vec<_> = [&left, &right]
                .iter()
                .map(|result| result.as_ref().err().map(|error| error.code().to_owned()))
                .collect();
            assert_eq!(
                codes.iter().filter(|code| code.is_none()).count(),
                1,
                "đúng một lần gỡ được: {codes:?}"
            );
            assert!(codes.contains(&Some("auth.identity_last_method".to_owned())), "{codes:?}");
            let left = identity::for_user(&tenant, user).await.unwrap();
            assert_eq!(left.len(), 1, "còn đúng một cách đăng nhập");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn go_lien_ket_cua_nguoi_khac_la_khong_tim_thay() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let oidc = service(&services);
        let (_, first, _) = two_links(&services, "ada2@example.com").await;
        let (other, _, _) = two_links(&services, "bob@example.com").await;
        let error = oidc.unlink(&services.tenant, other, first).await.unwrap_err();
        assert_eq!(error.code(), "auth.identity_not_found");
        let error = oidc.unlink(&services.tenant, Uuid::now_v7(), first).await.unwrap_err();
        assert_eq!(error.code(), "auth.identity_not_found");
    })
    .await
    .unwrap();
}
