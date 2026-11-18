use super::*;
use autoxide_testkit::TestDatabase;

fn account(email: &str, admin: bool) -> NewAccount {
    NewAccount {
        email: email.to_owned(),
        name: "Ada".to_owned(),
        password: Some("correct horse battery staple".to_owned()),
        admin,
    }
}

#[tokio::test]
async fn tao_tai_khoan_admin_co_moi_quyen() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let accounts: &dyn Accounts = services.users.as_ref();

        let admin = accounts.create(&tenant, account("ada@example.com", true)).await.unwrap();
        let plain = accounts.create(&tenant, account("bob@example.com", false)).await.unwrap();

        assert!(services.roles.principal(&tenant, admin.id).await.unwrap().has_all_permissions());
        let bob = services.roles.principal(&tenant, plain.id).await.unwrap();
        assert!(!bob.has_all_permissions());
        assert!(bob.permissions().is_empty());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tao_hong_thi_khong_de_lai_gi() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let accounts: &dyn Accounts = services.users.as_ref();
        accounts.create(&tenant, account("ada@example.com", true)).await.unwrap();

        let error = accounts.create(&tenant, account("ADA@example.com", true)).await.unwrap_err();

        assert_eq!(error.code(), "auth.email_taken");
        let page = services
            .users
            .list(
                &tenant,
                None,
                None,
                &PageRequest::first(10, autoxide_kernel::pagination::Order::Asc),
            )
            .await
            .unwrap();
        assert_eq!(page.data.len(), 1);
    })
    .await
    .unwrap();
}
