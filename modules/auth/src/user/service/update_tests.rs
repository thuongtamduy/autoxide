use super::*;
use autoxide_kernel::error::ErrorKind;
use autoxide_testkit::TestDatabase;

const PASSWORD: &str = "correct horse battery staple";

fn account(email: &str, admin: bool) -> NewAccount {
    NewAccount {
        email: email.to_owned(),
        name: "Ada".to_owned(),
        password: Some(PASSWORD.to_owned()),
        admin,
    }
}

fn disable(disabled: bool) -> UserChanges {
    UserChanges { name: None, disabled: Some(disabled), ..Default::default() }
}

#[tokio::test]
async fn doi_ten_khoa_va_mo_khoa() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let users = &services.users;
        Accounts::create(users.as_ref(), &tenant, account("root@example.com", true)).await.unwrap();
        let ada = Accounts::create(users.as_ref(), &tenant, account("ada@example.com", false))
            .await
            .unwrap();
        let tokens = services
            .sessions
            .login(&tenant, "ada@example.com", PASSWORD, None)
            .await
            .unwrap()
            .tokens();

        let renamed = users
            .update(
                &tenant,
                Actor::system(),
                ada.id,
                UserChanges {
                    name: Some("  Ada L. ".to_owned()),
                    disabled: None,
                    ..Default::default()
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(renamed.name, "Ada L.");
        assert!(renamed.is_active());

        let disabled =
            users.update(&tenant, Actor::system(), ada.id, disable(true), None).await.unwrap();
        assert!(!disabled.is_active());
        // Khoá lần nữa giữ thời điểm khoá cũ.
        let again =
            users.update(&tenant, Actor::system(), ada.id, disable(true), None).await.unwrap();
        assert_eq!(again.disabled_at, disabled.disabled_at);
        // Đăng xuất ở mọi nơi: refresh token cũ hết dùng được.
        let refresh =
            services.sessions.refresh(Some(&tenant), &tokens.refresh_token).await.unwrap_err();
        assert_eq!(refresh.code(), "auth.invalid_refresh_token");
        let login =
            services.sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap_err();
        assert_eq!(login.code(), "auth.user_disabled");

        let enabled =
            users.update(&tenant, Actor::system(), ada.id, disable(false), None).await.unwrap();
        assert!(enabled.is_active());
        services.sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();

        let missing = users
            .update(&tenant, Actor::system(), Uuid::now_v7(), disable(true), None)
            .await
            .unwrap_err();
        assert_eq!(missing.code(), "auth.user_not_found");
        let empty =
            UserChanges { name: Some(" ".to_owned()), disabled: None, ..Default::default() };
        assert_eq!(
            users.update(&tenant, Actor::system(), ada.id, empty, None).await.unwrap_err().code(),
            "validation_failed"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khong_khoa_duoc_admin_cuoi_cung() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let users = &services.users;
        let root = Accounts::create(users.as_ref(), &tenant, account("root@example.com", true))
            .await
            .unwrap();

        let error =
            users.update(&tenant, Actor::system(), root.id, disable(true), None).await.unwrap_err();

        assert_eq!(error.kind(), ErrorKind::Conflict);
        assert_eq!(error.code(), "auth.last_admin");
        assert!(users.get(&tenant, root.id).await.unwrap().unwrap().is_active());
        // Có admin khác còn hoạt động thì khoá được.
        Accounts::create(users.as_ref(), &tenant, account("second@example.com", true))
            .await
            .unwrap();
        users.update(&tenant, Actor::system(), root.id, disable(true), None).await.unwrap();
    })
    .await
    .unwrap();
}

/// Hai admin cuối cùng xoá (hay khoá) nhau cùng lúc: đúng một bên thành công, luôn còn
/// một admin hoạt động — trên cả hai database (D-149).
#[tokio::test]
async fn hai_admin_cuoi_xoa_nhau_cung_luc_van_con_mot() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let tenant = services.tenant.clone();
        let users = services.users.clone();
        let a = Accounts::create(users.as_ref(), &tenant, account("a@example.com", true))
            .await
            .unwrap();
        let b = Accounts::create(users.as_ref(), &tenant, account("b@example.com", true))
            .await
            .unwrap();
        let (first, second) = (users.clone(), users.clone());
        let (one, two) = (tenant.clone(), tenant.clone());
        let delete_a = tokio::spawn(async move { first.delete(&one, Actor::system(), a.id).await });
        let disable_b = tokio::spawn(async move {
            second.update(&two, Actor::system(), b.id, disable(true), None).await.map(|_| ())
        });
        let outcomes = [delete_a.await.unwrap(), disable_b.await.unwrap()];
        let refused: Vec<_> =
            outcomes.iter().filter_map(|outcome| outcome.as_ref().err()).collect();
        assert_eq!(refused.len(), 1, "{outcomes:?}");
        assert_eq!(refused[0].code(), "auth.last_admin");
        let left =
            crate::role::count_active_admins(services.database.write(), &tenant).await.unwrap();
        assert_eq!(left, 1);
    })
    .await
    .unwrap();
}
