use super::*;
use crate::user::NewUser;
use autoxide_kernel::error::ErrorKind;
use autoxide_testkit::TestDatabase;

const PASSWORD: &str = "correct horse battery staple";

/// Service đã có sẵn một người dùng `ada@example.com`.
async fn setup(
    test: &TestDatabase,
) -> (Arc<SessionService>, Arc<UserService>, User, TenantContext) {
    let (sessions, users, tenant) = crate::testing::sessions(test).await;
    let user = users
        .create(
            &tenant,
            Actor::system(),
            NewUser {
                email: "ada@example.com".to_owned(),
                name: "Ada".to_owned(),
                password: Some(PASSWORD.to_owned()),
            },
        )
        .await
        .unwrap();
    (sessions, users, user, tenant)
}

fn bearer(tokens: &Tokens) -> String {
    format!("Bearer {}", tokens.access_token)
}

/// Đăng xuất mọi phiên chạy cùng lúc với refresh của các phiên đó: không token mới
/// nào lọt qua — trên cả hai database.
#[tokio::test]
async fn dang_xuat_moi_phien_trong_luc_refresh() {
    TestDatabase::run(async |test| {
        let (sessions, _, user, tenant) = setup(test).await;
        let mut refresh_tokens = Vec::new();
        for _ in 0..4 {
            let tokens = sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap();
            refresh_tokens.push(tokens.tokens().refresh_token);
        }
        sessions.throttle.clear(&tenant, "ada@example.com").await.unwrap();

        let mut tasks = Vec::new();
        for token in refresh_tokens {
            let sessions = sessions.clone();
            let tenant = tenant.clone();
            tasks.push(tokio::spawn(async move {
                sessions.refresh(Some(&tenant), &token).await.map(|_| ())
            }));
        }
        let (revoker, in_task) = (sessions.clone(), tenant.clone());
        let revoke = tokio::spawn(async move {
            revoker.revoke_sessions(&in_task, Actor::user(user.id), user.id, None).await.map(|_| ())
        });
        for task in tasks {
            let _ = task.await.unwrap();
        }
        revoke.await.unwrap().unwrap();

        let page = PageRequest::first(10, Default::default());
        let live = sessions.list_sessions(&tenant, user.id, &page).await.unwrap();
        assert!(live.data.is_empty(), "{} sessions survived", live.data.len());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dang_nhap_roi_dung_access_token() {
    TestDatabase::run(async |test| {
        let (sessions, _, user, tenant) = setup(test).await;

        let tokens =
            sessions.login(&tenant, "ADA@example.com", PASSWORD, None).await.unwrap().tokens();

        assert_eq!(tokens.expires_in, 900);
        assert_ne!(tokens.refresh_token, tokens.access_token);
        assert_eq!(sessions.authenticate(&tenant, Some(&bearer(&tokens))).await.unwrap(), user);
        let wrong =
            sessions.login(&tenant, "ada@example.com", "wrong password", None).await.unwrap_err();
        assert_eq!(wrong.code(), "auth.invalid_credentials");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doan_sai_nhieu_lan_thi_khoa_ca_mat_khau_dung() {
    TestDatabase::run(async |test| {
        let (sessions, _, _, tenant) = setup(test).await;
        // Một lần đúng giữa chừng xoá các lần sai trước.
        for _ in 0..4 {
            sessions.login(&tenant, "ada@example.com", "wrong password", None).await.unwrap_err();
        }
        sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();

        for _ in 0..5 {
            let error = sessions
                .login(&tenant, "ada@example.com", "wrong password", None)
                .await
                .unwrap_err();
            assert_eq!(error.code(), "auth.invalid_credentials");
        }
        let locked = sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap_err();

        assert_eq!(locked.kind(), ErrorKind::TooManyRequests);
        assert_eq!(locked.code(), "auth.login_throttled");
        // Email không tồn tại cũng bị khoá y hệt: không lộ email nào có thật.
        for _ in 0..5 {
            sessions.login(&tenant, "ghost@example.com", "wrong password", None).await.unwrap_err();
        }
        let ghost =
            sessions.login(&tenant, "ghost@example.com", "wrong password", None).await.unwrap_err();
        assert_eq!(ghost.code(), "auth.login_throttled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn header_authorization_sai_hoac_thieu() {
    TestDatabase::run(async |test| {
        let (sessions, _, _, tenant) = setup(test).await;
        let tokens =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();

        let lower = format!("bearer {}", tokens.access_token);
        assert!(sessions.authenticate(&tenant, Some(&lower)).await.is_ok());
        for (header, code) in [
            (None, "auth.missing_token"),
            (Some("Bearer"), "auth.missing_token"),
            (Some("Basic YWRhOnB3"), "auth.missing_token"),
            (Some("Bearer not.a.token"), "auth.invalid_token"),
        ] {
            let error = sessions.authenticate(&tenant, header).await.unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Unauthenticated);
            assert_eq!(error.code(), code, "{header:?}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn refresh_xoay_vong_token() {
    TestDatabase::run(async |test| {
        let (sessions, _, user, tenant) = setup(test).await;
        let first =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();

        let second = sessions.refresh(Some(&tenant), &first.refresh_token).await.unwrap();
        let third = sessions.refresh(Some(&tenant), &second.refresh_token).await.unwrap();

        assert_ne!(second.refresh_token, first.refresh_token);
        assert_ne!(third.refresh_token, second.refresh_token);
        assert_eq!(sessions.authenticate(&tenant, Some(&bearer(&third))).await.unwrap(), user);
        let unknown = sessions.refresh(Some(&tenant), "no-such-token").await.unwrap_err();
        assert_eq!(unknown.code(), "auth.invalid_refresh_token");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dung_lai_refresh_token_thu_hoi_ca_phien() {
    TestDatabase::run(async |test| {
        let (sessions, _, _, tenant) = setup(test).await;
        let first =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();
        let second = sessions.refresh(Some(&tenant), &first.refresh_token).await.unwrap();
        // Một phiên khác của cùng người dùng không bị ảnh hưởng.
        let other =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();

        let reused = sessions.refresh(Some(&tenant), &first.refresh_token).await.unwrap_err();

        assert_eq!(reused.kind(), ErrorKind::Unauthenticated);
        assert_eq!(reused.code(), "auth.refresh_token_reused");
        // Token mới nhất của họ đó cũng mất hiệu lực: kẻ trộm không giữ được phiên.
        let revoked = sessions.refresh(Some(&tenant), &second.refresh_token).await.unwrap_err();
        assert_eq!(revoked.code(), "auth.invalid_refresh_token");
        assert!(sessions.refresh(Some(&tenant), &other.refresh_token).await.is_ok());
    })
    .await
    .unwrap();
}

/// Nhiều request đổi CÙNG một refresh token cùng lúc: đúng một bên thắng, mọi bên
/// khác bị coi là dùng lại, và cả họ bị thu hồi — trên cả hai database.
#[tokio::test]
async fn refresh_dong_thoi_chi_mot_ben_thang() {
    TestDatabase::run(async |test| {
        let (sessions, _, _, tenant) = setup(test).await;
        let tokens =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();

        let attempts: Vec<_> = (0..6)
            .map(|_| {
                let sessions = sessions.clone();
                let token = tokens.refresh_token.clone();
                let tenant = tenant.clone();
                tokio::spawn(async move { sessions.refresh(Some(&tenant), &token).await })
            })
            .collect();
        let mut winners = Vec::new();
        for attempt in attempts {
            match attempt.await.unwrap() {
                Ok(tokens) => winners.push(tokens),
                Err(error) => assert_eq!(error.code(), "auth.refresh_token_reused"),
            }
        }

        assert_eq!(winners.len(), 1);
        let winner = sessions.refresh(Some(&tenant), &winners[0].refresh_token).await.unwrap_err();
        assert_eq!(winner.code(), "auth.invalid_refresh_token");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dang_xuat_thu_hoi_phien_va_goi_lai_van_on() {
    TestDatabase::run(async |test| {
        let (sessions, _, _, tenant) = setup(test).await;
        let first =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();
        let second = sessions.refresh(Some(&tenant), &first.refresh_token).await.unwrap();

        sessions.logout(Some(&tenant), &first.refresh_token).await.unwrap();
        sessions.logout(Some(&tenant), &first.refresh_token).await.unwrap();
        sessions.logout(Some(&tenant), "no-such-token").await.unwrap();

        let error = sessions.refresh(Some(&tenant), &second.refresh_token).await.unwrap_err();
        assert_eq!(error.code(), "auth.invalid_refresh_token");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn refresh_token_het_han_khong_dung_duoc() {
    TestDatabase::run(async |test| {
        let (sessions, _, _, tenant) = setup(test).await;
        let tokens =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();
        Entity::update_many()
            .col_expr(Column::ExpiresAt, Expr::value(now()))
            .exec(tenant.database().write())
            .await
            .unwrap();

        let error = sessions.refresh(Some(&tenant), &tokens.refresh_token).await.unwrap_err();

        assert_eq!(error.code(), "auth.invalid_refresh_token");
    })
    .await
    .unwrap();
}

/// Kiểm mật khẩu rồi mới cấp: mật khẩu đổi xen giữa (đặt lại, khoá tài khoản) thì không cấp
/// được — `lock_grant` so hash vừa kiểm trong transaction cấp (D-156).
#[tokio::test]
async fn mat_khau_doi_giua_luc_kiem_va_luc_cap_thi_khong_cap() {
    TestDatabase::run(async |test| {
        let (sessions, users, user, tenant) = setup(test).await;
        let (_, verified) = sessions.check_password(&tenant, user.id, PASSWORD).await.unwrap();
        let write = tenant.database().write();
        assert!(lock_grant(write, &tenant, user.id, Some(&verified)).await.unwrap());

        users
            .set_password(&tenant, Actor::system(), user.id, "a brand new passphrase".to_owned())
            .await
            .unwrap();
        assert!(!lock_grant(write, &tenant, user.id, Some(&verified)).await.unwrap());
        assert!(lock_grant(write, &tenant, user.id, None).await.unwrap());

        let disable =
            crate::user::UserChanges { name: None, disabled: Some(true), ..Default::default() };
        users.update(&tenant, Actor::system(), user.id, disable, None).await.unwrap();
        assert!(!lock_grant(write, &tenant, user.id, None).await.unwrap());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_mat_khau_thu_hoi_moi_phien() {
    TestDatabase::run(async |test| {
        let (sessions, users, user, tenant) = setup(test).await;
        let first =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();
        let second =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();

        users
            .set_password(&tenant, Actor::system(), user.id, "a brand new passphrase".to_owned())
            .await
            .unwrap();

        for tokens in [first, second] {
            let error = sessions.refresh(Some(&tenant), &tokens.refresh_token).await.unwrap_err();
            assert_eq!(error.code(), "auth.invalid_refresh_token");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tai_khoan_bi_khoa_mat_quyen() {
    TestDatabase::run(async |test| {
        let (sessions, _, user, tenant) = setup(test).await;
        let tokens =
            sessions.login(&tenant, "ada@example.com", PASSWORD, None).await.unwrap().tokens();
        crate::user::entity_for_tests::disable(tenant.database().write(), user.id).await;

        let me = sessions.authenticate(&tenant, Some(&bearer(&tokens))).await.unwrap_err();
        let refresh = sessions.refresh(Some(&tenant), &tokens.refresh_token).await.unwrap_err();

        assert_eq!(me.code(), "auth.user_disabled");
        assert_eq!(refresh.code(), "auth.user_disabled");
    })
    .await
    .unwrap();
}
