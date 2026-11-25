//! Token SCIM không cấp, giữ hay gỡ thứ gì vượt quá quyền của người tạo token — tính lúc DÙNG token
//! (D-20261121-scim-claim-covers, D-20261125-scim-grant-cover). Qua REST thật, cả SQLite và Postgres.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::tests::{scim, scim_token, setup, user_body};
use crate::http::test_support::{PASSWORD, bearer, call};
use crate::testing::Services;
use crate::user::NewUser;

const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";

fn patch(operations: Value) -> Value {
    json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": operations})
}

/// Người `email` chỉ có `auth.scim.manage` (role `sync` ở gốc): trả header `Authorization` của
/// họ và một token SCIM họ tạo.
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
    let root = autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid();
    let boss = autoxide_kernel::access::Principal::with_all_permissions(uuid::Uuid::now_v7())
        .in_tenant(tenant.id());
    services
        .roles
        .set_memberships(tenant, &boss, user.id, vec![(root, vec![role.id])])
        .await
        .unwrap();
    let access = bearer(app, email, PASSWORD).await;
    let token = scim_token(app, &access, None).await;
    (access, token)
}

/// ID role `admin` của khách.
async fn admin_role(app: &TestApp, admin: &str) -> String {
    let (_, _, roles) = call(app, "GET", "/roles", Some(admin), None).await;
    roles["data"].as_array().unwrap().iter().find(|role| role["name"] == "admin").unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Nhóm `Admins` (do token của admin tạo) và luật "Admins → admin ở gốc" do admin đặt: trả ID
/// nhóm và ID luật.
async fn admin_group(
    app: &TestApp,
    services: &Services,
    admin: &str,
    admin_token: &str,
) -> (String, String) {
    let group = json!({"schemas": [GROUP_SCHEMA], "displayName": "Admins"});
    let (status, group) = scim(app, "POST", "/Groups", admin_token, Some(group)).await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let group_id = group["id"].as_str().unwrap().to_owned();
    let root = services.tenant.id().as_uuid();
    let body =
        json!({"group_id": group_id, "org_id": root, "role_id": admin_role(app, admin).await});
    let (status, _, mapping) =
        call(app, "POST", "/scim/group-mappings", Some(admin), Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{mapping}");
    (group_id, mapping["data"]["id"].as_str().unwrap().to_owned())
}

async fn scim_user(app: &TestApp, token: &str, email: &str) -> String {
    let (status, user) = scim(app, "POST", "/Users", token, Some(user_body(email, email))).await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    user["id"].as_str().unwrap().to_owned()
}

/// Người `user_id` có toàn quyền không (đọc lại từ database).
async fn is_admin(services: &Services, user_id: &str) -> bool {
    let id = uuid::Uuid::parse_str(user_id).unwrap();
    services.roles.principal(&services.tenant, id).await.unwrap().has_all_permissions()
}

fn assert_forbidden(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["schemas"], json!(["urn:ietf:params:scim:api:messages:2.0:Error"]), "{body}");
    assert_eq!(body["status"], "403", "{body}");
}

/// Thành viên nhóm (đọc bằng token `token`).
async fn members(app: &TestApp, token: &str, group_id: &str) -> Vec<String> {
    let (_, group) = scim(app, "GET", &format!("/Groups/{group_id}"), token, None).await;
    let mut ids: Vec<String> = group["members"]
        .as_array()
        .map(|members| {
            members.iter().map(|member| member["value"].as_str().unwrap().to_owned()).collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

#[tokio::test]
async fn token_quyen_thap_khong_dua_ai_vao_nhom_admin() {
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let (group_id, _) = admin_group(&app, &services, &admin, &admin_token).await;
        let path = format!("/Groups/{group_id}");
        let stranger = scim_user(&app, &low_token, "stranger@example.com").await;

        // PATCH add, PUT thay danh sách: đều 403, nhóm không đổi (không để lại "ý định" chờ đồng
        // bộ sau), người đó không có quyền gì.
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": stranger}]}]));
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(add)).await;
        assert_forbidden(status, &body);
        let put = json!({"schemas": [GROUP_SCHEMA], "displayName": "Admins", "members": [{"value": stranger}]});
        let (status, body) = scim(&app, "PUT", &path, &low_token, Some(put)).await;
        assert_forbidden(status, &body);
        assert!(members(&app, &admin_token, &group_id).await.is_empty());
        assert!(!is_admin(&services, &stranger).await);

        // Token của admin thì được; token quyền thấp không gỡ ra, không xoá nhóm được.
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": stranger}]}]));
        let (status, body) = scim(&app, "PATCH", &path, &admin_token, Some(add)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(is_admin(&services, &stranger).await);
        let remove =
            patch(json!([{"op": "remove", "path": format!("members[value eq \"{stranger}\"]")}]));
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(remove)).await;
        assert_forbidden(status, &body);
        let (status, body) = scim(&app, "DELETE", &path, &low_token, None).await;
        assert_forbidden(status, &body);
        assert!(is_admin(&services, &stranger).await);
        assert_eq!(members(&app, &admin_token, &group_id).await, [stranger.clone()]);

        // Đổi tên nhóm không đổi ai có quyền gì (luật gắn ID nhóm): được.
        let rename = patch(json!([{"op": "replace", "path": "displayName", "value": "Owners"}]));
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(rename)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(is_admin(&services, &stranger).await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quyen_nguoi_tao_tinh_luc_dung_token() {
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (group_id, _) = admin_group(&app, &services, &admin, &admin_token).await;
        // Admin thứ hai tạo token, rồi bị hạ quyền: token của họ không cấp admin được nữa.
        let input = NewUser {
            email: "second@example.com".to_owned(),
            name: "Second".to_owned(),
            password: Some(PASSWORD.to_owned()),
        };
        let second = services
            .users
            .create_with_roles(&services.tenant, Actor::system(), input, true)
            .await
            .unwrap();
        let second_access = bearer(&app, "second@example.com", PASSWORD).await;
        let second_token = scim_token(&app, &second_access, None).await;
        let (status, _, body) = call(
            &app,
            "PUT",
            &format!("/users/{}/roles", second.id),
            Some(&admin),
            Some(json!({"role_ids": []})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let stranger = scim_user(&app, &second_token, "stranger@example.com").await;
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": stranger}]}]));
        let (status, body) =
            scim(&app, "PATCH", &format!("/Groups/{group_id}"), &second_token, Some(add)).await;
        assert_forbidden(status, &body);
        assert!(!is_admin(&services, &stranger).await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_quyen_thap_khong_xoa_tai_khoan_quyen_cao() {
    TestDatabase::run(async |test| {
        let (app, services, _, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let target = scim_user(&app, &admin_token, "chief@example.com").await;
        let id = uuid::Uuid::parse_str(&target).unwrap();
        crate::role::grant_admin(services.database.write(), &services.tenant, id).await.unwrap();

        let path = format!("/Users/{target}");
        let (status, body) = scim(&app, "DELETE", &path, &low_token, None).await;
        assert_forbidden(status, &body);
        assert!(services.users.get(&services.tenant, id).await.unwrap().is_some());

        // Tài khoản thường: token quyền thấp xoá được; tài khoản quyền cao: token của admin xoá được.
        let plain = scim_user(&app, &low_token, "plain@example.com").await;
        let (status, _) = scim(&app, "DELETE", &format!("/Users/{plain}"), &low_token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = scim(&app, "DELETE", &path, &admin_token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_quyen_thap_khong_go_luat_anh_xa_admin() {
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (low_access, _) = sync_operator(&app, &services, "it@example.com").await;
        let (group_id, mapping_id) = admin_group(&app, &services, &admin, &admin_token).await;
        let member = scim_user(&app, &admin_token, "member@example.com").await;
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": member}]}]));
        let (status, body) =
            scim(&app, "PATCH", &format!("/Groups/{group_id}"), &admin_token, Some(add)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(is_admin(&services, &member).await);

        let path = format!("/scim/group-mappings/{mapping_id}");
        let (status, _, body) = call(&app, "DELETE", &path, Some(&low_access), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["code"], "auth.role_beyond_you", "{body}");
        assert!(is_admin(&services, &member).await);
        let (_, _, mappings) = call(&app, "GET", "/scim/group-mappings", Some(&admin), None).await;
        assert_eq!(mappings["data"].as_array().unwrap().len(), 1, "{mappings}");

        let (status, _, _) = call(&app, "DELETE", &path, Some(&admin), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!is_admin(&services, &member).await);
    })
    .await
    .unwrap();
}
