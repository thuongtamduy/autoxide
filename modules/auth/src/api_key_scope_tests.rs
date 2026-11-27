//! UT-2: API key quyền tối thiểu không xem, không thu hồi được API key khác của cùng chủ — cả qua REST lẫn tool MCP
//! (cùng service). Nó chỉ thấy và tự thu hồi được chính nó. Người có `auth.api_keys.manage` thu hồi key của người khác
//! chỉ khi bao trùm quyền của chủ key. Chạy trên SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::{Value, json};

use crate::http::test_support::{PASSWORD, app, bearer, call};
use crate::user::NewUser;

async fn key(
    app: &autoxide_testkit::TestApp,
    session: &str,
    name: &str,
    permissions: Value,
) -> (String, String) {
    let body = json!({"name": name, "current_password": PASSWORD, "permissions": permissions});
    let (status, _, created) = call(app, "POST", "/api-keys", Some(session), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    (
        created["data"]["id"].as_str().unwrap().to_owned(),
        created["data"]["key"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn api_key_khong_thu_hoi_hay_xem_key_khac_qua_rest_va_mcp() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let boss = NewUser {
            email: "boss@example.com".to_owned(),
            name: "Boss".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services
            .users
            .create_with_roles(&services.tenant, Actor::system(), boss, true)
            .await
            .unwrap();
        let session = bearer(&app, "boss@example.com", PASSWORD).await;
        let (strong_id, strong) =
            key(&app, &session, "deploy", json!(["auth.users.read", "auth.roles.read"])).await;
        let (weak_id, weak) = key(&app, &session, "probe", json!(["auth.users.read"])).await;
        let (other_id, _) = key(&app, &session, "spare", json!(["auth.users.read"])).await;
        let with_weak = format!("Bearer {weak}");

        // REST: key yếu chỉ thấy chính nó, không thu hồi được key khác (404 như key không có).
        let (status, _, list) = call(&app, "GET", "/api-keys", Some(&with_weak), None).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let ids: Vec<&str> = list["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [weak_id.as_str()]);
        let (status, _, refused) =
            call(&app, "DELETE", &format!("/api-keys/{strong_id}"), Some(&with_weak), None).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("auth.api_key_not_found"))
        );

        // MCP: cùng kết quả.
        let client = app.mcp(&weak).await.unwrap();
        let answer =
            client.call_tool(tool("auth.revoke_api_key", json!({"id": other_id}))).await.unwrap();
        assert_eq!(answer.is_error, Some(true), "{answer:?}");
        assert_eq!(answer.structured_content.unwrap()["code"], "auth.api_key_not_found");
        let answer = client.call_tool(tool("auth.list_api_keys", json!({}))).await.unwrap();
        let listed = answer.structured_content.unwrap();
        assert_eq!(listed["data"].as_array().unwrap().len(), 1, "{listed}");
        assert_eq!(listed["data"][0]["id"], weak_id);
        client.cancel().await.unwrap();

        // Key khác vẫn chạy.
        let (status, _, _) =
            call(&app, "GET", "/users", Some(&format!("Bearer {strong}")), None).await;
        assert_eq!(status, StatusCode::OK);

        // Key lộ tự tắt được chính nó (MCP), rồi thôi dùng được.
        let client = app.mcp(&weak).await.unwrap();
        let answer =
            client.call_tool(tool("auth.revoke_api_key", json!({"id": weak_id}))).await.unwrap();
        assert_ne!(answer.is_error, Some(true), "{answer:?}");
        client.cancel().await.unwrap();
        let (status, _, _) = call(&app, "GET", "/users", Some(&with_weak), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Phiên đăng nhập của chủ vẫn quản lý được mọi key của mình.
        let (status, _, list) = call(&app, "GET", "/api-keys", Some(&session), None).await;
        assert_eq!((status, list["data"].as_array().unwrap().len()), (StatusCode::OK, 2));
        let (status, _, _) =
            call(&app, "DELETE", &format!("/api-keys/{other_id}"), Some(&session), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quan_ly_key_khong_thu_hoi_key_cua_nguoi_quyen_cao_hon() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let tenant = services.tenant.clone();
        let boss = NewUser {
            email: "boss@example.com".to_owned(),
            name: "Boss".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create_with_roles(&tenant, Actor::system(), boss, true).await.unwrap();
        let helper = NewUser {
            email: "helper@example.com".to_owned(),
            name: "Helper".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let helper = services.users.create(&tenant, Actor::system(), helper).await.unwrap();
        let input = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            grants: Vec::new(),
            name: "key-admin".to_owned(),
            description: String::new(),
            permissions: vec!["auth.api_keys.manage".to_owned(), "auth.users.read".to_owned()],
        };
        let role = services.roles.create(&tenant, Actor::system(), input).await.unwrap();
        services
            .roles
            .set_user_roles(&tenant, Actor::system(), helper.id, vec![role.id])
            .await
            .unwrap();
        let boss_session = bearer(&app, "boss@example.com", PASSWORD).await;
        let helper_session = bearer(&app, "helper@example.com", PASSWORD).await;
        let (boss_key, _) = key(&app, &boss_session, "deploy", json!(["auth.users.read"])).await;
        let (helper_key, helper_secret) =
            key(&app, &helper_session, "ops", json!(["auth.api_keys.manage", "auth.users.read"]))
                .await;

        // REST và MCP: người quản lý key không bao trùm quyền của chủ key (admin) → 403.
        let (status, _, refused) =
            call(&app, "DELETE", &format!("/all-api-keys/{boss_key}"), Some(&helper_session), None)
                .await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("auth.target_has_more_permissions"))
        );
        let (status, _, refused) = call(
            &app,
            "DELETE",
            &format!("/all-api-keys/{boss_key}"),
            Some(&format!("Bearer {helper_secret}")),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        // Key của chính mình và của admin bởi admin: được.
        let (status, _, _) =
            call(&app, "DELETE", &format!("/all-api-keys/{boss_key}"), Some(&boss_session), None)
                .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _, _) = call(
            &app,
            "DELETE",
            &format!("/all-api-keys/{helper_key}"),
            Some(&helper_session),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

/// Kiểm độc lập #572: token OAuth (client làm thay người dùng) không xem, không thu hồi, không xoay key nào; API key có
/// `auth.api_keys.manage` không thu hồi được key khác của chính chủ qua `/all-api-keys`; xoay key bằng API key bị từ
/// chối cùng một lỗi dù id có tồn tại hay không.
#[tokio::test]
async fn token_oauth_va_api_key_quan_ly_khong_vuot_pham_vi() {
    TestDatabase::run(async |test| {
        use autoxide_kernel::access::OAuthAccess;
        use autoxide_kernel::pagination::{Order, PageRequest};
        let (app, services) = app(test).await;
        let tenant = services.tenant.clone();
        let boss = NewUser {
            email: "boss@example.com".to_owned(),
            name: "Boss".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let boss =
            services.users.create_with_roles(&tenant, Actor::system(), boss, true).await.unwrap();
        let session = bearer(&app, "boss@example.com", PASSWORD).await;
        let (deploy_id, _) = key(&app, &session, "deploy", json!(["auth.users.read"])).await;
        let (manager_id, _) =
            key(&app, &session, "ops", json!(["auth.api_keys.manage", "auth.users.read"])).await;
        let deploy: uuid::Uuid = deploy_id.parse().unwrap();
        let manager: uuid::Uuid = manager_id.parse().unwrap();
        let (_, principal) =
            services.roles.user_principal(&tenant, boss.id).await.unwrap().unwrap();
        let first = PageRequest::first(10, Order::Desc);

        // Token OAuth: danh sách rỗng, thu hồi 404, xoay 403.
        let oauth = principal.clone().with_oauth(OAuthAccess::new(
            uuid::Uuid::now_v7(),
            "https://client.example/app.json",
            "https://api.example.com/api/v1",
        ));
        assert!(services.api_keys.list_own(&tenant, &oauth, &first).await.unwrap().data.is_empty());
        let error = services.api_keys.revoke(&tenant, &oauth, deploy).await.unwrap_err();
        assert_eq!(error.code(), "auth.api_key_not_found");

        // API key quản lý key: không thu hồi key khác của chính chủ qua `revoke_any` (không bao trùm quyền chủ).
        // Như bộ xác thực dựng cho key: chỉ các quyền của key.
        let via_manager = autoxide_kernel::access::Principal::new(
            boss.id,
            ["auth.api_keys.manage".to_owned(), "auth.users.read".to_owned()],
        )
        .in_tenant(tenant.id())
        .via_api_key(manager);
        let error = services.api_keys.revoke_any(&tenant, &via_manager, deploy).await.unwrap_err();
        assert_eq!(error.code(), "auth.target_has_more_permissions");

        // Xoay bằng API key: cùng lỗi cho key có thật và id không có.
        for id in [deploy, uuid::Uuid::now_v7()] {
            let rotation = crate::api_key::Rotation {
                overlap_hours: 0,
                expires_at: None,
                current_password: PASSWORD.to_owned(),
                second_factor: None,
            };
            let error =
                services.api_keys.rotate(&tenant, &via_manager, id, rotation).await.unwrap_err();
            assert_eq!(error.code(), "auth.api_key_cannot_create_keys");
        }

        // Phiên đăng nhập của chủ: vẫn thu hồi được key của mình qua `revoke_any`; key còn nguyên tới lúc đó.
        let (status, _, list) = call(&app, "GET", "/api-keys", Some(&session), None).await;
        assert_eq!((status, list["data"].as_array().unwrap().len()), (StatusCode::OK, 2));
        let (status, _, _) =
            call(&app, "DELETE", &format!("/all-api-keys/{deploy}"), Some(&session), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}
