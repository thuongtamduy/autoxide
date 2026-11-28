//! Người chịu trách nhiệm cho dữ liệu SCIM (P16, D-20261128-scim-owners): thành viên do token quyền thấp đưa vào nhóm
//! chờ duyệt khi admin đặt luật sau; đồng bộ đưa tài khoản lên cao hơn người tạo liên kết thì thu hồi mọi cách vào cũ.
//! Qua REST thật, cả SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::tests::{scim, scim_token, setup, user_body};
use crate::http::test_support::{PASSWORD, bearer, call};
use crate::testing::Services;
use crate::user::NewUser;

const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";

fn add_member(user_id: &str) -> Value {
    json!({
        "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
        "Operations": [{"op": "add", "path": "members", "value": [{"value": user_id}]}],
    })
}

/// Người `email` chỉ có `auth.scim.manage` ở gốc: header `Authorization` và một token SCIM họ tạo.
async fn sync_operator(app: &TestApp, services: &Services, email: &str) -> (String, String) {
    let tenant = &services.tenant;
    let input = crate::role::RoleInput {
        denies: Vec::new(),
        requires_mfa: false,
        name: "sync".to_owned(),
        description: String::new(),
        permissions: vec!["auth.scim.manage".to_owned()],
        grants: Vec::new(),
    };
    let role = services.roles.create(tenant, Actor::system(), input).await.unwrap();
    let input = NewUser {
        email: email.to_owned(),
        name: "Sync".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    let user = services.users.create(tenant, Actor::system(), input).await.unwrap();
    services.roles.set_user_roles(tenant, Actor::system(), user.id, vec![role.id]).await.unwrap();
    let access = bearer(app, email, PASSWORD).await;
    let token = scim_token(app, &access, None).await;
    (access, token)
}

async fn admin_role(app: &TestApp, admin: &str) -> String {
    let (_, _, roles) = call(app, "GET", "/roles", Some(admin), None).await;
    roles["data"].as_array().unwrap().iter().find(|role| role["name"] == "admin").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Tạo User bằng token `token`, đặt mật khẩu cho nó: trả ID.
async fn user_with_password(
    app: &TestApp,
    services: &Services,
    token: &str,
    email: &str,
) -> String {
    let (status, user) = scim(app, "POST", "/Users", token, Some(user_body(email, email))).await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    let id = user["id"].as_str().unwrap().to_owned();
    services
        .users
        .write_password(
            &services.tenant,
            Actor::system(),
            uuid::Uuid::parse_str(&id).unwrap(),
            "password",
            PASSWORD.to_owned(),
            crate::user::PasswordWrite::Initial,
        )
        .await
        .unwrap();
    id
}

async fn is_admin(services: &Services, user_id: &str) -> bool {
    let id = uuid::Uuid::parse_str(user_id).unwrap();
    services.roles.principal(&services.tenant, id).await.unwrap().has_all_permissions()
}

async fn can_sign_in(app: &TestApp, email: &str) -> bool {
    let login = json!({"email": email, "password": PASSWORD});
    call(app, "POST", "/login", None, Some(login)).await.0 == StatusCode::OK
}

#[tokio::test]
async fn thanh_vien_do_token_quyen_thap_dua_vao_cho_duyet() {
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (low, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let root = services.tenant.id().as_uuid();
        // Token quyền thấp: tạo nhóm chưa có luật, đưa X vào (được: chưa có luật thì không cấp gì).
        let group = json!({"schemas": [GROUP_SCHEMA], "displayName": "Ops"});
        let (status, group) = scim(&app, "POST", "/Groups", &low_token, Some(group)).await;
        assert_eq!(status, StatusCode::CREATED, "{group}");
        let group_id = group["id"].as_str().unwrap().to_owned();
        let x = user_with_password(&app, &services, &low_token, "x@example.com").await;
        let path = format!("/Groups/{group_id}");
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(add_member(&x))).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // Admin đặt luật "Ops → admin": X không thành admin, nằm trong `waiting_user_ids`.
        let rule = json!({"group_id": group_id, "org_id": root, "role_id": admin_role(&app, &admin).await});
        let (status, _, mapped) =
            call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(rule)).await;
        assert_eq!(status, StatusCode::CREATED, "{mapped}");
        assert_eq!(mapped["data"]["granted_user_ids"], json!([]), "{mapped}");
        assert_eq!(mapped["data"]["waiting_user_ids"], json!([x]), "{mapped}");
        assert!(!is_admin(&services, &x).await);
        // Token admin gửi lại thành viên nhóm, token quyền thấp gửi lại: X vẫn chờ.
        for token in [&admin_token, &low_token] {
            scim(&app, "PATCH", &path, token, Some(add_member(&x))).await;
            assert!(!is_admin(&services, &x).await);
        }

        // Người quyền thấp không duyệt được (không cấp được `admin`); người lạ / nhóm lạ: 404.
        let approve = format!("/scim/groups/{group_id}/members/{x}/approve");
        let (status, _, refused) = call(&app, "POST", &approve, Some(&low), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert_eq!(refused["code"], "auth.role_beyond_you", "{refused}");
        let stranger = uuid::Uuid::now_v7();
        let missing = format!("/scim/groups/{group_id}/members/{stranger}/approve");
        let (status, _, body) = call(&app, "POST", &missing, Some(&admin), None).await;
        assert_eq!((status, body["code"].as_str()), (StatusCode::NOT_FOUND, Some("auth.scim_member_not_found")));
        let missing = format!("/scim/groups/{stranger}/members/{x}/approve");
        let (status, _, body) = call(&app, "POST", &missing, Some(&admin), None).await;
        assert_eq!((status, body["code"].as_str()), (StatusCode::NOT_FOUND, Some("auth.scim_group_not_found")));
        assert!(can_sign_in(&app, "x@example.com").await, "chưa đổi gì");

        // Admin duyệt: X thành admin; tài khoản do token quyền thấp tạo nên mọi cách vào cũ bị thu hồi trước.
        let (status, _, approved) = call(&app, "POST", &approve, Some(&admin), None).await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        assert_eq!(approved["data"]["synced"], true, "{approved}");
        assert!(is_admin(&services, &x).await);
        assert!(!can_sign_in(&app, "x@example.com").await, "mật khẩu đặt trước khi lên admin bị bỏ");
        // Duyệt lại: được, không đổi gì.
        let (status, _, approved) = call(&app, "POST", &approve, Some(&admin), None).await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        assert!(is_admin(&services, &x).await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nang_tai_khoan_len_cao_hon_nguoi_tao_lien_ket_thu_hoi_cua_vao_cu() {
    // Token quyền thấp tạo trước tài khoản (kẻ tấn công đặt mật khẩu), hệ thống danh tính KHÔNG đổi email (biến thể
    // (i) của D-20261125-scim-sync-report mục 7) mà đưa thẳng vào nhóm admin bằng token admin: mật khẩu cũ chết.
    // Tài khoản do chính token admin tạo thì không bị thu hồi gì.
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let root = services.tenant.id().as_uuid();
        let group = json!({"schemas": [GROUP_SCHEMA], "displayName": "Admins"});
        let (_, group) = scim(&app, "POST", "/Groups", &admin_token, Some(group)).await;
        let group_id = group["id"].as_str().unwrap().to_owned();
        let rule = json!({"group_id": group_id, "org_id": root, "role_id": admin_role(&app, &admin).await});
        let (status, _, mapped) =
            call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(rule)).await;
        assert_eq!(status, StatusCode::CREATED, "{mapped}");
        let path = format!("/Groups/{group_id}");

        let planted = user_with_password(&app, &services, &low_token, "planted@example.com").await;
        let own = user_with_password(&app, &services, &admin_token, "own@example.com").await;
        let session = bearer(&app, "planted@example.com", PASSWORD).await;
        for id in [&planted, &own] {
            let (status, body) = scim(&app, "PATCH", &path, &admin_token, Some(add_member(id))).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert!(is_admin(&services, id).await);
        }
        assert!(!can_sign_in(&app, "planted@example.com").await);
        assert_eq!(call(&app, "GET", "/me", Some(&session), None).await.0, StatusCode::UNAUTHORIZED);
        assert!(can_sign_in(&app, "own@example.com").await, "người tạo liên kết tự cấp: không thu hồi");
    })
    .await
    .unwrap();
}
