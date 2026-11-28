//! Nhóm SCIM → role ở nút (bước 66) qua REST, cả SQLite và Postgres.

use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use serde_json::{Value, json};

use super::tests::{scim, setup, urlencode, user_body};
use crate::http::test_support::{PASSWORD, bearer, call};

fn patch(operations: Value) -> Value {
    json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": operations})
}

/// Tên các role người `user_id` đang giữ ở nút gốc.
async fn roles_of(app: &autoxide_testkit::TestApp, admin: &str, user_id: &str) -> Vec<String> {
    let (status, _, body) =
        call(app, "GET", &format!("/users/{user_id}/roles"), Some(admin), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut names: Vec<String> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|role| role["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn nhom_doi_thi_membership_doi() {
    TestDatabase::run(async |test| {
        let (app, services, admin, token) = setup(test).await;
        let root = services.tenant.id().as_uuid().to_string();
        let (_, _, editor) = call(
            &app,
            "POST",
            "/roles",
            Some(&admin),
            Some(json!({"name": "editor", "permissions": ["notes.write"]})),
        )
        .await;
        let editor = editor["data"]["id"].as_str().unwrap().to_owned();
        let (_, a) =
            scim(&app, "POST", "/Users", &token, Some(user_body("a@example.com", "a"))).await;
        let (_, b) =
            scim(&app, "POST", "/Users", &token, Some(user_body("b@example.com", "b"))).await;
        let (a, b) = (a["id"].as_str().unwrap().to_owned(), b["id"].as_str().unwrap().to_owned());

        // Nhóm chưa có luật: vào hộp việc, chưa ai có role.
        let group = json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
            "displayName": "Editors",
            "externalId": "grp-1",
            "members": [{"value": a}],
        });
        let (status, created) = scim(&app, "POST", "/Groups", &token, Some(group)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let group_id = created["id"].as_str().unwrap().to_owned();
        assert_eq!(created["members"][0]["value"], a);
        assert!(roles_of(&app, &admin, &a).await.is_empty());
        let response = app
            .get("/api/v1/kernel/work/counts")
            .header("authorization", &admin)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.json()["data"]["counts"]["auth.scim_unmapped_groups"],
            1,
            "{}",
            response.json()
        );

        // Luật: Editors → editor ở nút gốc. Thành viên hiện có nhận role ngay.
        let body = json!({"group_id": group_id, "org_id": root, "role_id": editor});
        let (status, _, mapping) =
            call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(body.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{mapping}");
        let mapping_id = mapping["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(roles_of(&app, &admin, &a).await, ["editor"]);
        let (status, _, again) =
            call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(body)).await;
        assert_eq!(
            (status, again["code"].as_str()),
            (StatusCode::CONFLICT, Some("auth.scim_mapping_taken"))
        );
        // (Số đếm hộp việc được kernel giữ 10 giây: xem trang quản trị nhóm.)
        let (_, _, groups) = call(&app, "GET", "/scim/groups", Some(&admin), None).await;
        let counts =
            (groups["data"][0]["members"].as_u64(), groups["data"][0]["mappings"].as_u64());
        assert_eq!(counts, (Some(1), Some(1)), "{groups}");

        // Thêm b, bỏ a (cách Entra ID gửi): role đi theo.
        let body = patch(json!([
            {"op": "Add", "path": "members", "value": [{"value": b}]},
            {"op": "Remove", "path": "members", "value": [{"value": a}]},
        ]));
        let (status, changed) =
            scim(&app, "PATCH", &format!("/Groups/{group_id}"), &token, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert!(roles_of(&app, &admin, &a).await.is_empty());
        assert_eq!(roles_of(&app, &admin, &b).await, ["editor"]);
        let (_, user) = scim(&app, "GET", &format!("/Users/{b}"), &token, None).await;
        assert_eq!(user["groups"][0]["display"], "Editors");

        // Role gán tay không bị SCIM gỡ: b giữ editor bằng tay, rời nhóm vẫn còn.
        let (status, _, _) = call(
            &app,
            "PUT",
            &format!("/users/{b}/roles"),
            Some(&admin),
            Some(json!({"role_ids": [editor]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let body = patch(json!([{"op": "remove", "path": format!("members[value eq \"{b}\"]")}]));
        let (status, _) =
            scim(&app, "PATCH", &format!("/Groups/{group_id}"), &token, Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(roles_of(&app, &admin, &b).await, ["editor"]);

        // Lọc nhóm theo tên / thành viên, bỏ thành viên khi được xin.
        let body = patch(json!([{"op": "add", "path": "members", "value": [{"value": a}]}]));
        scim(&app, "PATCH", &format!("/Groups/{group_id}"), &token, Some(body)).await;
        let path = format!(
            "/Groups?filter={}&excludedAttributes=members",
            urlencode("displayName eq \"editors\"")
        );
        let (_, found) = scim(&app, "GET", &path, &token, None).await;
        assert_eq!(found["totalResults"], 1);
        assert!(found["Resources"][0].get("members").is_none());
        let path = format!("/Groups?filter={}", urlencode(&format!("members eq \"{a}\"")));
        assert_eq!(scim(&app, "GET", &path, &token, None).await.1["totalResults"], 1);

        // Bỏ luật: a mất role. Xoá nhóm: không còn gì.
        let (status, _, _) =
            call(&app, "DELETE", &format!("/scim/group-mappings/{mapping_id}"), Some(&admin), None)
                .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(roles_of(&app, &admin, &a).await.is_empty());
        let (status, _) = scim(&app, "DELETE", &format!("/Groups/{group_id}"), &token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            scim(&app, "GET", &format!("/Groups/{group_id}"), &token, None).await.0,
            StatusCode::NOT_FOUND
        );
        let (_, _, groups) = call(&app, "GET", "/scim/groups", Some(&admin), None).await;
        assert_eq!(groups["data"], json!([]));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn luat_anh_xa_khong_dung_de_tu_nang_quyen() {
    TestDatabase::run(async |test| {
        let (app, services, admin, token) = setup(test).await;
        // Một người chỉ có `auth.scim.manage`: không đặt được luật cấp `admin`.
        let (_, _, scim_role) =
            call(&app, "POST", "/roles", Some(&admin), Some(json!({"name": "sync", "permissions": ["auth.scim.manage"]}))).await;
        let scim_role = scim_role["data"]["id"].as_str().unwrap().to_owned();
        let ada = services.users.find_by_email(&services.tenant, "ada@example.com").await.unwrap().unwrap().id;
        call(&app, "PUT", &format!("/users/{ada}/roles"), Some(&admin), Some(json!({"role_ids": [scim_role]}))).await;
        let ada_token = bearer(&app, "ada@example.com", PASSWORD).await;
        let (_, _, roles) = call(&app, "GET", "/roles", Some(&admin), None).await;
        let admin_role = roles["data"].as_array().unwrap().iter().find(|role| role["name"] == "admin").unwrap()["id"].clone();
        let group = json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"], "displayName": "Admins"});
        let (_, group) = scim(&app, "POST", "/Groups", &token, Some(group)).await;
        let root = services.tenant.id().as_uuid();
        let body = json!({"group_id": group["id"], "org_id": root, "role_id": admin_role});
        let (status, _, refused) = call(&app, "POST", "/scim/group-mappings", Some(&ada_token), Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert_eq!(refused["code"], "auth.role_beyond_you", "{refused}");
        assert_eq!(refused["errors"][0]["code"], "more_than_you_have", "{refused}");
        // Nhóm không có thì 404.
        let body = json!({"group_id": uuid::Uuid::now_v7(), "org_id": root, "role_id": admin_role});
        let (status, _, missing) = call(&app, "POST", "/scim/group-mappings", Some(&admin), Some(body)).await;
        assert_eq!((status, missing["code"].as_str()), (StatusCode::NOT_FOUND, Some("auth.scim_group_not_found")));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_patch_thanh_vien_cung_luc_khong_mat_ai() {
    // Đọc nhóm → áp PATCH → ghi: khoá dòng nhóm nên hai lần thêm cùng lúc không đè nhau (D-181).
    TestDatabase::run(async |test| {
        let (app, _, _, token) = setup(test).await;
        let mut ids = Vec::new();
        for name in ["x@example.com", "y@example.com"] {
            let (_, user) = scim(&app, "POST", "/Users", &token, Some(user_body(name, name))).await;
            ids.push(user["id"].as_str().unwrap().to_owned());
        }
        let group = json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"], "displayName": "Ops"});
        let (_, group) = scim(&app, "POST", "/Groups", &token, Some(group)).await;
        let path = format!("/Groups/{}", group["id"].as_str().unwrap());
        let add = |id: &str| patch(json!([{"op": "add", "path": "members", "value": [{"value": id}]}]));
        let (left, right) = tokio::join!(
            scim(&app, "PATCH", &path, &token, Some(add(&ids[0]))),
            scim(&app, "PATCH", &path, &token, Some(add(&ids[1]))),
        );
        assert_eq!((left.0, right.0), (StatusCode::OK, StatusCode::OK), "{left:?} {right:?}");
        let (_, group) = scim(&app, "GET", &path, &token, None).await;
        assert_eq!(group["members"].as_array().unwrap().len(), 2, "{group}");
    })
    .await
    .unwrap();
}
