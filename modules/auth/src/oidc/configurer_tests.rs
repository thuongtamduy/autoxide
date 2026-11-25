//! Vai "người cấu hình" của nhà cung cấp đăng nhập ngoài (quyết định có tự gộp vào tài khoản có sẵn
//! không) không sống lâu hơn quyền của người đó (D-20261125-durable-grants): mất `auth.oidc.manage`
//! hay bị khoá hết cửa vào thì nhà cung cấp ngừng tự gộp. Cả SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;

use super::tests::{app, claims, fake_server, oidc_body, provider, sign_in, token};
use crate::http::test_support::PASSWORD;
use crate::user::NewUser;

#[tokio::test]
async fn nguoi_cau_hinh_bi_khoa_het_cua_vao_thi_nha_cung_cap_khong_tu_gop() {
    // Tài khoản cắm sẵn P tạm có `auth.oidc.manage`, dựng nhà cung cấp với issuer kẻ tấn công
    // kiểm soát; sau đó P bị khoá hết cửa vào (đổi email qua SCIM, "không phải tôi"…) rồi được trao
    // toàn quyền. Issuer đó khai email đã xác minh của admin: không được gộp vào admin.
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, _) = app(test, true).await;
        let tenant = services.tenant.clone();
        let input = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "sso".to_owned(),
            description: String::new(),
            permissions: vec!["auth.oidc.manage".to_owned()],
            grants: Vec::new(),
        };
        let sso = services.roles.create(&tenant, Actor::system(), input).await.unwrap();
        let planted = NewUser {
            email: "planted@example.com".to_owned(),
            name: "P".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let planted = services.users.create(&tenant, Actor::system(), planted).await.unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), planted.id, vec![sso.id])
            .await
            .unwrap();
        let planted_token = token(&app, "planted@example.com").await;
        provider(&app, &planted_token, oidc_body(&fake, "evil", false)).await;
        let root =
            services.users.find_by_email(&tenant, "root@example.com").await.unwrap().unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .apply_verified_email(&tenant, &transaction, root.id, &root.email, Timestamp::now())
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        // Khoá hết cửa vào của P (như khi SCIM đổi email), rồi P được trao toàn quyền.
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .lock_out_in(&tenant, &transaction, Actor::system(), planted.id)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        crate::role::grant_admin(services.database.write(), &tenant, planted.id).await.unwrap();

        let (status, body) =
            sign_in(&app, &fake, "evil", claims(&fake, "evil", "s-root", "root@example.com", true))
                .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "auth.oidc_link_required", "{body}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_cau_hinh_mat_quyen_quan_ly_oidc_thi_nha_cung_cap_khong_tu_gop() {
    // Người cấu hình còn role `editor` nhưng mất `auth.oidc.manage`: không gộp vào tài khoản `editor`.
    TestDatabase::run(async |test| {
        let fake = fake_server().await;
        let (app, services, _) = app(test, true).await;
        let tenant = services.tenant.clone();
        let role = |name: &str, permissions: &[&str]| crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: name.to_owned(),
            description: String::new(),
            permissions: permissions.iter().map(|permission| (*permission).to_owned()).collect(),
            grants: Vec::new(),
        };
        let sso = services
            .roles
            .create(&tenant, Actor::system(), role("sso", &["auth.oidc.manage"]))
            .await
            .unwrap();
        let editor = services
            .roles
            .create(&tenant, Actor::system(), role("editor", &["notes.write"]))
            .await
            .unwrap();
        let manager = NewUser {
            email: "sso@example.com".to_owned(),
            name: "SSO".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let manager = services.users.create(&tenant, Actor::system(), manager).await.unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), manager.id, vec![sso.id, editor.id])
            .await
            .unwrap();
        let manager_token = token(&app, "sso@example.com").await;
        provider(&app, &manager_token, oidc_body(&fake, "idp", false)).await;
        let ada = services.users.find_by_email(&tenant, "ada@example.com").await.unwrap().unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), ada.id, vec![editor.id])
            .await
            .unwrap();
        let transaction = tenant.database().begin_write().await.unwrap();
        services
            .users
            .apply_verified_email(&tenant, &transaction, ada.id, &ada.email, Timestamp::now())
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        services
            .roles
            .set_user_roles(&tenant, Actor::system(), manager.id, vec![editor.id])
            .await
            .unwrap();
        let (status, body) =
            sign_in(&app, &fake, "idp", claims(&fake, "idp", "s-ada", "ada@example.com", true))
                .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["code"], "auth.oidc_link_required", "{body}");
    })
    .await
    .unwrap();
}
