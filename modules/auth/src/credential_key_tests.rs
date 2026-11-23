//! Pentest hộp đen 2026-11: một API key (dù mang `auth.scim.manage` / `auth.oidc.manage`) không
//! được tạo token SCIM hay thêm, sửa nhà cung cấp đăng nhập — cả qua REST lẫn MCP. Hai thứ đó tin
//! theo quyền của NGƯỜI tạo, không theo phạm vi key, nên key hẹp sẽ thành đường vào tài khoản quyền
//! cao. Chạy trên SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::json;

use crate::http::test_support::{PASSWORD, app, bearer, call};
use crate::user::NewUser;

#[tokio::test]
async fn api_key_khong_tao_duoc_token_scim_hay_nha_cung_cap_dang_nhap() {
    TestDatabase::run(async |test| {
        let (app, services) = app(test).await;
        let boss = NewUser {
            email: "boss@example.com".to_owned(),
            name: "Boss".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        services.users.create_with_roles(&services.tenant, Actor::system(), boss, true).await.unwrap();
        let session = bearer(&app, "boss@example.com", PASSWORD).await;
        let body = json!({"name": "idp", "current_password": PASSWORD,
                          "permissions": ["auth.scim.manage", "auth.oidc.manage"]});
        let (status, _, created) = call(&app, "POST", "/api-keys", Some(&session), Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let key = created["data"]["key"].as_str().unwrap().to_owned();
        let with_key = format!("Bearer {key}");

        // Token SCIM: key bị từ chối, phiên đăng nhập vẫn tạo được.
        let name = json!({"name": "Entra ID"});
        let (status, _, refused) =
            call(&app, "POST", "/scim/tokens", Some(&with_key), Some(name.clone())).await;
        assert_eq!((status, refused["code"].as_str()), (StatusCode::FORBIDDEN, Some("auth.session_required")));
        let (status, _, made) = call(&app, "POST", "/scim/tokens", Some(&session), Some(name)).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");

        // Nhà cung cấp đăng nhập: REST và tool MCP cùng từ chối key.
        let provider = json!({"slug": "evil", "kind": "oidc", "name": "Evil", "client_id": "x",
                              "issuer": "https://idp.example.com", "redirect_uris": ["https://app.example.com/cb"]});
        let (status, _, refused) =
            call(&app, "POST", "/oidc-providers", Some(&with_key), Some(provider.clone())).await;
        assert_eq!((status, refused["code"].as_str()), (StatusCode::FORBIDDEN, Some("auth.session_required")), "{refused}");
        let client = app.mcp(&key).await.unwrap();
        let answer = client.call_tool(tool("auth.create_oidc_provider", provider)).await.unwrap();
        assert_eq!(answer.is_error, Some(true), "{answer:?}");
        let problem = answer.structured_content.unwrap();
        assert_eq!(problem["code"], "auth.session_required", "{problem}");
        client.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
