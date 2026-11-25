//! L-7 (rà soát bảo mật 2026-11): đổi mật khẩu (không phải khôi phục) mặc định giữ API key và liên kết
//! đăng nhập ngoài; `revoke_other_credentials: true` gỡ hết như khôi phục mật khẩu. Chạy trên SQLite
//! và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::http::test_support::{PASSWORD, app, bearer, call};
use crate::testing::Services;
use crate::user::NewUser;

const NEW_PASSWORD: &str = "a brand new long password";

// Tài khoản quản trị `boss` có một API key và một liên kết đăng nhập ngoài; trả (session, key, id).
async fn boss_with_credentials(
    app: &autoxide_testkit::TestApp,
    services: &Services,
) -> (String, String, Uuid) {
    let boss = NewUser {
        email: "boss@example.com".to_owned(),
        name: "Boss".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    let user = services
        .users
        .create_with_roles(&services.tenant, Actor::system(), boss, true)
        .await
        .unwrap();
    let session = bearer(app, "boss@example.com", PASSWORD).await;
    let body =
        json!({"name": "script", "current_password": PASSWORD, "permissions": ["auth.users.read"]});
    let (status, _, created) = call(app, "POST", "/api-keys", Some(&session), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let key = format!("Bearer {}", created["data"]["key"].as_str().unwrap());
    let external = crate::oidc::identity::External {
        subject: "sub-1".to_owned(),
        email: Some("boss@example.com".to_owned()),
        email_verified: true,
        name: None,
    };
    let write = services.tenant.database().write();
    crate::oidc::identity::insert(write, &services.tenant, user.id, Uuid::now_v7(), &external)
        .await
        .unwrap();
    (session, key, user.id)
}

async fn change(app: &autoxide_testkit::TestApp, session: &str, body: Value) {
    let (status, _, changed) = call(app, "POST", "/me/password", Some(session), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{changed}");
}

#[tokio::test]
async fn doi_mat_khau_mac_dinh_giu_api_key_va_lien_ket() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let (session, key, user) = boss_with_credentials(&app, &services).await;
        let body = json!({"current_password": PASSWORD, "new_password": NEW_PASSWORD});
        change(&app, &session, body).await;

        let (status, _, _) = call(&app, "GET", "/me", Some(&key), None).await;
        assert_eq!(status, StatusCode::OK, "không chọn thu hồi thì API key vẫn dùng được");
        let linked = crate::oidc::identity::for_user(&services.tenant, user).await.unwrap();
        assert_eq!(linked.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn doi_mat_khau_chon_thu_hoi_thi_go_api_key_va_lien_ket() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let (session, key, user) = boss_with_credentials(&app, &services).await;
        let body = json!({"current_password": PASSWORD, "new_password": NEW_PASSWORD,
                          "revoke_other_credentials": true});
        change(&app, &session, body).await;

        let (status, _, _) = call(&app, "GET", "/me", Some(&key), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "API key bị thu hồi");
        let linked = crate::oidc::identity::for_user(&services.tenant, user).await.unwrap();
        assert!(linked.is_empty(), "liên kết đăng nhập ngoài bị gỡ");
    })
    .await
    .unwrap();
}
