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
        assert_eq!(members(&app, &admin_token, &group_id).await, std::slice::from_ref(&stranger));

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
        assert_eq!(creator_status(&app, &admin, second.id).await, "missing_permission");
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

#[tokio::test]
async fn patch_nhieu_op_mot_op_vuot_quyen_thi_khong_ghi_gi() {
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let (group_id, _) = admin_group(&app, &services, &admin, &admin_token).await;
        let path = format!("/Groups/{group_id}");
        let first = scim_user(&app, &admin_token, "first@example.com").await;
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": first}]}]));
        assert_eq!(scim(&app, "PATCH", &path, &admin_token, Some(add)).await.0, StatusCode::OK);
        let second = scim_user(&app, &low_token, "second@example.com").await;

        // Đổi tên (hợp lệ) + thêm người (vượt quyền): không op nào được ghi.
        let ops = patch(json!([
            {"op": "replace", "path": "displayName", "value": "Owners"},
            {"op": "add", "path": "members", "value": [{"value": second}]},
        ]));
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(ops)).await;
        assert_forbidden(status, &body);
        // Bớt một người, thêm người khác trong cùng một PATCH: cũng không ghi gì.
        let ops = patch(json!([
            {"op": "remove", "path": format!("members[value eq \"{first}\"]")},
            {"op": "add", "path": "members", "value": [{"value": second}]},
        ]));
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(ops)).await;
        assert_forbidden(status, &body);

        let (_, group) = scim(&app, "GET", &path, &admin_token, None).await;
        assert_eq!(group["displayName"], "Admins", "{group}");
        assert_eq!(members(&app, &admin_token, &group_id).await, std::slice::from_ref(&first));
        assert!(is_admin(&services, &first).await);
        assert!(!is_admin(&services, &second).await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn put_user_doi_email_tai_khoan_admin_bi_chan() {
    TestDatabase::run(async |test| {
        let (app, services, _, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let target = scim_user(&app, &admin_token, "chief@example.com").await;
        let id = uuid::Uuid::parse_str(&target).unwrap();
        crate::role::grant_admin(services.database.write(), &services.tenant, id).await.unwrap();
        let mut body = user_body("chief@example.com", "chief@example.com");
        body["emails"] =
            json!([{"value": "attacker@example.com", "type": "work", "primary": true}]);
        let (status, refused) =
            scim(&app, "PUT", &format!("/Users/{target}"), &low_token, Some(body)).await;
        assert_forbidden(status, &refused);
        let account = services.users.get(&services.tenant, id).await.unwrap().unwrap();
        assert_eq!(account.email, "chief@example.com");
    })
    .await
    .unwrap();
}

/// `creator_status` của token theo `GET /scim/tokens`.
async fn creator_status(app: &TestApp, admin: &str, created_by: uuid::Uuid) -> String {
    let (status, _, tokens) = call(app, "GET", "/scim/tokens", Some(admin), None).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    let created_by = created_by.to_string();
    let token = tokens["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|token| token["created_by"] == created_by.as_str())
        .unwrap();
    token["creator_status"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn token_cua_nguoi_tao_bi_khoa_hay_bi_xoa() {
    use sea_orm::{ColumnTrait as _, EntityTrait as _, QueryFilter as _};
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let tenant = &services.tenant;
        let (group_id, _) = admin_group(&app, &services, &admin, &admin_token).await;
        let path = format!("/Groups/{group_id}");
        // Người có permission (để thử khoá) và người không có gì (để thử tạo).
        let (_, _) = sync_operator(&app, &services, "it@example.com").await;
        let it = services.users.find_by_email(tenant, "it@example.com").await.unwrap().unwrap();
        let admin_id =
            services.users.find_by_email(tenant, "boss@example.com").await.unwrap().unwrap().id;
        assert_eq!(creator_status(&app, &admin, admin_id).await, "active");

        let mut tokens = Vec::new();
        for email in ["gone@example.com", "locked@example.com"] {
            let input = NewUser {
                email: email.to_owned(),
                name: "Admin".to_owned(),
                password: Some(PASSWORD.to_owned()),
            };
            let user = services
                .users
                .create_with_roles(tenant, Actor::system(), input, true)
                .await
                .unwrap();
            let access = bearer(&app, email, PASSWORD).await;
            tokens.push((user.id, scim_token(&app, &access, None).await));
        }
        let (gone, gone_token) = tokens.remove(0);
        let (locked, locked_token) = tokens.remove(0);
        services.users.delete(tenant, Actor::system(), gone).await.unwrap();
        crate::user::entity::Entity::update_many()
            .col_expr(
                crate::user::entity::Column::DisabledAt,
                sea_orm::sea_query::Expr::value(Some(autoxide_kernel::time::Timestamp::now())),
            )
            .filter(crate::user::entity::Column::Id.eq(locked))
            .exec(services.database.write())
            .await
            .unwrap();
        assert_eq!(creator_status(&app, &admin, gone).await, "deleted");
        assert_eq!(creator_status(&app, &admin, locked).await, "disabled");

        // Liên kết tài khoản `it` (có permission) với SCIM bằng token của admin, để thử khoá.
        let (status, linked) =
            scim(&app, "POST", "/Users", &admin_token, Some(user_body("it@example.com", "it")))
                .await;
        assert_eq!(status, StatusCode::CREATED, "{linked}");
        for (label, token) in [("deleted", &gone_token), ("disabled", &locked_token)] {
            let stranger = scim_user(&app, token, &format!("{label}@example.com")).await;
            let add =
                patch(json!([{"op": "add", "path": "members", "value": [{"value": stranger}]}]));
            let (status, body) = scim(&app, "PATCH", &path, token, Some(add)).await;
            assert_forbidden(status, &body);
            assert!(!is_admin(&services, &stranger).await, "{label}");
            let deactivate = patch(json!([{"op": "replace", "value": {"active": false}}]));
            let (status, body) =
                scim(&app, "PATCH", &format!("/Users/{}", it.id), token, Some(deactivate)).await;
            assert_forbidden(status, &body);
        }
        assert!(services.users.get(tenant, it.id).await.unwrap().unwrap().disabled_at.is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn them_thanh_vien_dong_thoi_voi_dat_luat_khong_de_lai_thanh_vien_cho() {
    // Khoá dòng nhóm ở cả hai phía: lần thêm thành viên và lần đặt luật nối đuôi nhau. Sau cả hai,
    // người được token quyền thấp đưa vào hoặc không ở trong nhóm, hoặc đã nhận role qua luật của
    // admin và có tên trong `granted_user_ids` — không có thành viên "chờ" (ở nhóm có luật mà không
    // có role) để lần đồng bộ sau biến thành admin.
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let root = services.tenant.id().as_uuid();
        let admin_role = admin_role(&app, &admin).await;
        for round in 0..4 {
            let group = json!({"schemas": [GROUP_SCHEMA], "displayName": format!("G{round}")});
            let (_, group) = scim(&app, "POST", "/Groups", &admin_token, Some(group)).await;
            let group_id = group["id"].as_str().unwrap().to_owned();
            let stranger = scim_user(&app, &low_token, &format!("x{round}@example.com")).await;
            let add =
                patch(json!([{"op": "add", "path": "members", "value": [{"value": stranger}]}]));
            let path = format!("/Groups/{group_id}");
            let mapping = json!({"group_id": group_id, "org_id": root, "role_id": admin_role});
            let (patched, mapped) = tokio::join!(
                scim(&app, "PATCH", &path, &low_token, Some(add)),
                call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(mapping)),
            );
            assert_eq!(mapped.0, StatusCode::CREATED, "{}", mapped.2);
            let in_group = members(&app, &admin_token, &group_id).await.contains(&stranger);
            assert_eq!(in_group, patched.0 == StatusCode::OK, "{patched:?}");
            if !in_group {
                assert_forbidden(patched.0, &patched.1);
            }
            assert_eq!(is_admin(&services, &stranger).await, in_group, "vòng {round}");
            let granted = mapped.2["data"]["granted_user_ids"].as_array().unwrap();
            assert_eq!(granted.contains(&json!(stranger)), in_group, "vòng {round}: {}", mapped.2);
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dat_luat_cho_khoa_nhom_cua_lan_ghi_thanh_vien() {
    // Ép đúng thứ tự xen kẽ: một transaction đóng vai `update_group` của token quyền thấp (khoá dòng
    // nhóm, thấy nhóm chưa có luật, ghi thành viên X) và CHƯA commit; trong lúc đó admin đặt luật
    // "nhóm → admin". Đặt luật phải chờ khoá nhóm, rồi thấy X và cấp cho X (có tên trong
    // `granted_user_ids`). Không chờ thì luật commit trước, không thấy X, X thành thành viên chờ.
    use sea_orm::{ActiveModelTrait as _, EntityTrait as _, Set};
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let tenant = &services.tenant;
        let group = json!({"schemas": [GROUP_SCHEMA], "displayName": "Late"});
        let (_, group) = scim(&app, "POST", "/Groups", &admin_token, Some(group)).await;
        let group_id = uuid::Uuid::parse_str(group["id"].as_str().unwrap()).unwrap();
        let stranger = scim_user(&app, &low_token, "stranger@example.com").await;
        let stranger_id = uuid::Uuid::parse_str(&stranger).unwrap();

        // Đọc mọi thứ cần trước khi giữ transaction ghi (SQLite một kết nối ghi).
        let admin_role = admin_role(&app, &admin).await;
        let body =
            json!({"group_id": group_id, "org_id": tenant.id().as_uuid(), "role_id": admin_role});
        let transaction = tenant.database().begin_write().await.unwrap();
        autoxide_kernel::rows::lock_one(
            &transaction,
            super::entity::group::Entity::find_by_id((tenant.id().as_uuid(), group_id)),
        )
        .await
        .unwrap()
        .unwrap();
        super::entity::member::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            id: Set(uuid::Uuid::now_v7()),
            group_id: Set(group_id),
            user_id: Set(stranger_id),
            created_at: Set(autoxide_kernel::time::Timestamp::now()),
        }
        .insert(&transaction)
        .await
        .unwrap();

        let mapping = call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(body));
        let finished = std::sync::atomic::AtomicBool::new(false);
        let (mapped, ()) = tokio::join!(
            async {
                let result = mapping.await;
                finished.store(true, std::sync::atomic::Ordering::SeqCst);
                result
            },
            async {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                assert!(
                    !finished.load(std::sync::atomic::Ordering::SeqCst),
                    "đặt luật không chờ khoá nhóm của lần ghi thành viên"
                );
                transaction.commit().await.unwrap();
            },
        );
        let (status, _, mapped) = mapped;
        assert_eq!(status, StatusCode::CREATED, "{mapped}");
        assert_eq!(mapped["data"]["granted_user_ids"], json!([stranger]), "{mapped}");
        assert!(is_admin(&services, &stranger).await);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_quyen_thap_khong_doi_user_name_tai_khoan_quyen_cao() {
    // PT-19: `userName` / `externalId` là khoá hệ thống danh tính dùng để khớp người.
    TestDatabase::run(async |test| {
        let (app, services, _, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let target = scim_user(&app, &admin_token, "chief@example.com").await;
        let id = uuid::Uuid::parse_str(&target).unwrap();
        crate::role::grant_admin(services.database.write(), &services.tenant, id).await.unwrap();
        let path = format!("/Users/{target}");
        for operation in [
            json!({"op": "replace", "path": "userName", "value": "attacker@example.com"}),
            json!({"op": "replace", "path": "externalId", "value": "attacker"}),
            json!({"op": "replace", "path": "displayName", "value": "Someone else"}),
        ] {
            let (status, body) =
                scim(&app, "PATCH", &path, &low_token, Some(patch(json!([operation])))).await;
            assert_forbidden(status, &body);
        }
        let (_, user) = scim(&app, "GET", &path, &admin_token, None).await;
        assert_eq!(user["userName"], "chief@example.com", "{user}");
        assert_eq!(user["externalId"], "chief@example.com", "{user}");
        assert_eq!(user["displayName"], "Grace Hopper", "{user}");
        // PUT gửi lại đúng hồ sơ hiện có (không đổi gì): không bị chặn.
        let same = user_body("chief@example.com", "chief@example.com");
        let (status, body) = scim(&app, "PUT", &path, &low_token, Some(same)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // Tài khoản thường: token quyền thấp đổi được.
        let plain = scim_user(&app, &low_token, "plain@example.com").await;
        let rename =
            patch(json!([{"op": "replace", "path": "userName", "value": "plain2@example.com"}]));
        let (status, body) =
            scim(&app, "PATCH", &format!("/Users/{plain}"), &low_token, Some(rename)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chiem_truoc_user_name_khong_giu_duoc_tai_khoan_khi_email_doi() {
    // Token quyền thấp tạo trước User mang `userName` mà hệ thống danh tính SẼ dùng cho một người
    // (`chief@corp.example`) nhưng với email của kẻ tấn công; kẻ tấn công đặt mật khẩu. Hệ thống
    // danh tính (token của admin) khớp theo `userName`, đổi email về người thật rồi đưa vào nhóm
    // admin: kẻ tấn công không còn đường vào nào.
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let (group_id, _) = admin_group(&app, &services, &admin, &admin_token).await;
        let mut body = user_body("chief@corp.example", "chief");
        body["emails"] = json!([{"value": "attacker@evil.example", "type": "work", "primary": true}]);
        let (status, planted) = scim(&app, "POST", "/Users", &low_token, Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{planted}");
        let id = planted["id"].as_str().unwrap().to_owned();
        let user_id = uuid::Uuid::parse_str(&id).unwrap();
        services
            .users
            .write_password(
                &services.tenant,
                Actor::system(),
                user_id,
                "password",
                PASSWORD.to_owned(),
                crate::user::PasswordWrite::Initial,
            )
            .await
            .unwrap();
        let attacker = bearer(&app, "attacker@evil.example", PASSWORD).await;
        assert_eq!(call(&app, "GET", "/me", Some(&attacker), None).await.0, StatusCode::OK);
        // Tạm cấp cho tài khoản cắm sẵn quyền quản lý SCIM, tạo token SCIM T2, rồi gỡ quyền.
        let input = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "sync2".to_owned(),
            description: String::new(),
            permissions: vec!["auth.scim.manage".to_owned()],
            grants: Vec::new(),
        };
        let sync2 = services.roles.create(&services.tenant, Actor::system(), input).await.unwrap();
        services
            .roles
            .set_user_roles(&services.tenant, Actor::system(), user_id, vec![sync2.id])
            .await
            .unwrap();
        let planted_token = scim_token(&app, &attacker, None).await;
        services.roles.set_user_roles(&services.tenant, Actor::system(), user_id, vec![]).await.unwrap();

        // Hệ thống danh tính khớp theo `userName`, đổi email về người thật, đưa vào nhóm admin.
        let email = patch(json!([
            {"op": "replace", "path": "emails[type eq \"work\"].value", "value": "chief@corp.example"}
        ]));
        let (status, body) =
            scim(&app, "PATCH", &format!("/Users/{id}"), &admin_token, Some(email)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": id}]}]));
        let (status, body) =
            scim(&app, "PATCH", &format!("/Groups/{group_id}"), &admin_token, Some(add)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(is_admin(&services, &id).await);

        // Token SCIM tài khoản đó tạo đã bị thu hồi cùng lúc đổi email (không thì nay nó mạnh bằng admin).
        let (status, _) = scim(&app, "GET", "/Users", &planted_token, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // Phiên cũ chết; mật khẩu kẻ tấn công đặt không còn dùng được với email nào.
        assert_eq!(call(&app, "GET", "/me", Some(&attacker), None).await.0, StatusCode::UNAUTHORIZED);
        for email in ["attacker@evil.example", "chief@corp.example"] {
            let login = json!({"email": email, "password": PASSWORD});
            let (status, _, body) = call(&app, "POST", "/login", None, Some(login)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{email}: {body}");
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nguoi_tao_mat_quyen_quan_ly_scim_thi_token_khong_cap_duoc_gi() {
    // Người tạo còn role `editor` nhưng mất `auth.scim.manage`: token không cấp được cả `editor`.
    TestDatabase::run(async |test| {
        let (app, services, admin, admin_token) = setup(test).await;
        let tenant = &services.tenant;
        let editor = crate::role::RoleInput {
            denies: Vec::new(),
            requires_mfa: false,
            name: "editor".to_owned(),
            description: String::new(),
            permissions: vec!["notes.write".to_owned()],
            grants: Vec::new(),
        };
        let editor = services.roles.create(tenant, Actor::system(), editor).await.unwrap();
        let (_, low_token) = sync_operator(&app, &services, "it@example.com").await;
        let it = services.users.find_by_email(tenant, "it@example.com").await.unwrap().unwrap();
        let mut roles: Vec<uuid::Uuid> = services
            .roles
            .user_roles(tenant, it.id)
            .await
            .unwrap()
            .into_iter()
            .map(|role| role.id)
            .collect();
        roles.push(editor.id);
        services.roles.set_user_roles(tenant, Actor::system(), it.id, roles).await.unwrap();
        let group = json!({"schemas": [GROUP_SCHEMA], "displayName": "Editors"});
        let (_, group) = scim(&app, "POST", "/Groups", &admin_token, Some(group)).await;
        let group_id = group["id"].as_str().unwrap().to_owned();
        let body =
            json!({"group_id": group_id, "org_id": tenant.id().as_uuid(), "role_id": editor.id});
        let (status, _, mapped) =
            call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{mapped}");
        let path = format!("/Groups/{group_id}");
        let first = scim_user(&app, &low_token, "first@example.com").await;
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": first}]}]));
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(add)).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // Mất `auth.scim.manage`, còn `editor`.
        services
            .roles
            .set_user_roles(tenant, Actor::system(), it.id, vec![editor.id])
            .await
            .unwrap();
        let second = scim_user(&app, &low_token, "second@example.com").await;
        let add = patch(json!([{"op": "add", "path": "members", "value": [{"value": second}]}]));
        let (status, body) = scim(&app, "PATCH", &path, &low_token, Some(add)).await;
        assert_forbidden(status, &body);
    })
    .await
    .unwrap();
}
