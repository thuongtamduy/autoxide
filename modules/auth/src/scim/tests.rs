//! Đồng bộ nhân viên qua SCIM (bước 66) qua REST với module `auth` thật, cả SQLite và Postgres:
//! token SCIM, `Users` (lọc, tạo, thay, PATCH, xoá), khoá và thu hồi khi `active = false`.

use autoxide_kernel::event::Actor;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::http::test_support::{PASSWORD, app, bearer, call};
use crate::testing::Services;
use crate::user::NewUser;

pub(crate) const BASE: &str = "/api/v1/auth/scim/v2";

/// App, service, header `Authorization` của một admin, và một token SCIM của khách mặc định.
pub(crate) async fn setup(test: &TestDatabase) -> (TestApp, Services, String, String) {
    let (app, services) = app(test).await;
    let input = NewUser {
        email: "boss@example.com".to_owned(),
        name: "Boss".to_owned(),
        password: Some(PASSWORD.to_owned()),
    };
    services.users.create_with_roles(&services.tenant, Actor::system(), input, true).await.unwrap();
    let admin = bearer(&app, "boss@example.com", PASSWORD).await;
    let token = scim_token(&app, &admin, None).await;
    (app, services, admin, token)
}

/// Tạo token SCIM bằng REST quản trị (ở khách `tenant` khi có).
pub(crate) async fn scim_token(app: &TestApp, admin: &str, tenant: Option<&str>) -> String {
    let mut request = app.post("/api/v1/auth/scim/tokens").header("authorization", admin);
    if let Some(slug) = tenant {
        request = request.header("autoxide-tenant", slug);
    }
    let response = request.json(json!({"name": "Entra ID"})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{}", response.json());
    let body = response.json();
    assert_eq!(body["data"]["scim_base_path"], BASE);
    body["data"]["token_secret"].as_str().unwrap().to_owned()
}

/// Gọi endpoint SCIM với token `token`; thân gửi dạng `application/scim+json`.
pub(crate) async fn scim(
    app: &TestApp,
    method: &str,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app
        .request(method, &format!("{BASE}{path}"))
        .header("authorization", &format!("Bearer {token}"));
    if let Some(body) = body {
        request = request.header("content-type", "application/scim+json").bytes(body.to_string());
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    if !response.body().is_empty() {
        let content_type = response.header("content-type").to_owned();
        assert_eq!(content_type, "application/scim+json", "{}", response.text());
    }
    (status, if response.body().is_empty() { Value::Null } else { response.json() })
}

pub(crate) fn user_body(user_name: &str, external_id: &str) -> Value {
    json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
        "userName": user_name,
        "externalId": external_id,
        "name": {"givenName": "Grace", "familyName": "Hopper"},
        "emails": [{"value": user_name, "type": "work", "primary": true}],
        "active": true,
    })
}

fn assert_scim_error(
    status: StatusCode,
    body: &Value,
    wanted: StatusCode,
    scim_type: Option<&str>,
) {
    assert_eq!(status, wanted, "{body}");
    assert_eq!(body["schemas"], json!(["urn:ietf:params:scim:api:messages:2.0:Error"]), "{body}");
    assert_eq!(body["status"], wanted.as_u16().to_string());
    match scim_type {
        Some(scim_type) => assert_eq!(body["scimType"], scim_type, "{body}"),
        None => assert!(body.get("scimType").is_none(), "{body}"),
    }
}

#[tokio::test]
async fn tao_doc_va_kham_pha_theo_rfc() {
    TestDatabase::run(async |test| {
        let (app, _, _, token) = setup(test).await;
        let (status, created) =
            scim(&app, "POST", "/Users", &token, Some(user_body("Grace@Example.com", "e-1"))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().unwrap().to_owned();
        assert_eq!(created["userName"], "Grace@Example.com");
        assert_eq!(created["displayName"], "Grace Hopper");
        assert_eq!(created["emails"][0]["value"], "grace@example.com");
        assert_eq!(created["active"], true);
        assert_eq!(created["meta"]["location"], format!("{BASE}/Users/{id}"));
        let (status, got) = scim(&app, "GET", &format!("/Users/{id}"), &token, None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(got["externalId"], "e-1");
        let (status, missing) = scim(&app, "GET", "/Users/not-an-id", &token, None).await;
        assert_scim_error(status, &missing, StatusCode::NOT_FOUND, None);
        // Thân không phải JSON.
        let response = app
            .post(&format!("{BASE}/Users"))
            .header("authorization", &format!("Bearer {token}"))
            .header("content-type", "application/scim+json")
            .bytes("{")
            .send()
            .await
            .unwrap();
        assert_scim_error(
            response.status(),
            &response.json(),
            StatusCode::BAD_REQUEST,
            Some("invalidSyntax"),
        );
        // Thiếu `userName`.
        let (status, body) =
            scim(&app, "POST", "/Users", &token, Some(json!({"active": true}))).await;
        assert_scim_error(status, &body, StatusCode::BAD_REQUEST, Some("invalidValue"));

        // Khám phá: không cần token.
        let config = app.get(&format!("{BASE}/ServiceProviderConfig")).send().await.unwrap();
        assert_eq!(config.status(), StatusCode::OK);
        assert_eq!(config.json()["patch"]["supported"], true);
        assert_eq!(config.json()["bulk"]["supported"], false);
        let types = app.get(&format!("{BASE}/ResourceTypes")).send().await.unwrap().json();
        assert_eq!(types["totalResults"], 2);
        let schemas = app.get(&format!("{BASE}/Schemas")).send().await.unwrap().json();
        assert_eq!(schemas["Resources"][0]["id"], "urn:ietf:params:scim:schemas:core:2.0:User");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn loc_eq_co_sw_dung() {
    TestDatabase::run(async |test| {
        let (app, _, _, token) = setup(test).await;
        for (name, external) in [
            ("alice@corp.example", "A-1"),
            ("alan@corp.example", "A-2"),
            ("bob@other.example", "B-1"),
        ] {
            let (status, body) =
                scim(&app, "POST", "/Users", &token, Some(user_body(name, external))).await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
        }
        let names = async |filter: &str| -> Vec<String> {
            let path = format!("/Users?filter={}", urlencode(filter));
            let (status, body) = scim(&app, "GET", &path, &token, None).await;
            assert_eq!(status, StatusCode::OK, "{filter}: {body}");
            assert_eq!(
                body["schemas"],
                json!(["urn:ietf:params:scim:api:messages:2.0:ListResponse"])
            );
            let mut names: Vec<String> = body["Resources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|user| user["userName"].as_str().unwrap().to_owned())
                .collect();
            assert_eq!(body["totalResults"], names.len());
            names.sort();
            names
        };
        // `userName` không phân biệt hoa thường (cách Entra ID / Okta tìm người trước khi tạo).
        assert_eq!(names("userName eq \"ALICE@corp.example\"").await, ["alice@corp.example"]);
        assert_eq!(names("userName sw \"al\"").await, ["alan@corp.example", "alice@corp.example"]);
        assert_eq!(names("userName co \"OTHER\"").await, ["bob@other.example"]);
        assert_eq!(names("userName ew \".example\" and externalId sw \"a-\"").await.len(), 2);
        // `externalId eq` phân biệt hoa thường (caseExact).
        assert_eq!(names("externalId eq \"A-2\"").await, ["alan@corp.example"]);
        assert!(names("externalId eq \"a-2\"").await.is_empty());
        assert_eq!(names("emails co \"@corp\"").await.len(), 2);
        assert_eq!(
            names("emails.value eq \"bob@other.example\" or userName eq \"alan@corp.example\"")
                .await
                .len(),
            2
        );
        assert_eq!(names("active eq true").await.len(), 3);
        assert_eq!(names("userName co \"%\"").await.len(), 0);

        // Phân trang `startIndex` / `count`.
        let (_, page) = scim(&app, "GET", "/Users?startIndex=2&count=1", &token, None).await;
        assert_eq!(
            (
                page["totalResults"].as_u64(),
                page["startIndex"].as_u64(),
                page["itemsPerPage"].as_u64()
            ),
            (Some(3), Some(2), Some(1))
        );
        let (_, page) = scim(&app, "GET", "/Users?count=0", &token, None).await;
        assert_eq!(page["totalResults"], 3);
        assert_eq!(page["Resources"], json!([]));

        for bad in ["title eq \"x\"", "userName gt \"a\"", "userName eq", "active co true"] {
            let path = format!("/Users?filter={}", urlencode(bad));
            let (status, body) = scim(&app, "GET", &path, &token, None).await;
            assert_scim_error(status, &body, StatusCode::BAD_REQUEST, Some("invalidFilter"));
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn external_id_trung_bi_tu_choi() {
    // Tên trong đặc tả: `externalId_trung_bi_tu_choi` (tên hàm Rust phải snake_case).
    TestDatabase::run(async |test| {
        let (app, _, _, token) = setup(test).await;
        let (status, _) =
            scim(&app, "POST", "/Users", &token, Some(user_body("one@example.com", "dup"))).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, body) =
            scim(&app, "POST", "/Users", &token, Some(user_body("two@example.com", "dup"))).await;
        assert_scim_error(status, &body, StatusCode::CONFLICT, Some("uniqueness"));
        let (status, body) =
            scim(&app, "POST", "/Users", &token, Some(user_body("ONE@example.com", "other"))).await;
        assert_scim_error(status, &body, StatusCode::CONFLICT, Some("uniqueness"));
        // PUT lên externalId của người khác cũng bị từ chối.
        let (_, two) =
            scim(&app, "POST", "/Users", &token, Some(user_body("two@example.com", "two"))).await;
        let id = two["id"].as_str().unwrap();
        let (status, body) = scim(
            &app,
            "PUT",
            &format!("/Users/{id}"),
            &token,
            Some(user_body("two@example.com", "dup")),
        )
        .await;
        assert_scim_error(status, &body, StatusCode::CONFLICT, Some("uniqueness"));

        // Hai request tạo cùng `externalId` cùng lúc: đúng một bên thắng (D-181).
        let (left, right) = tokio::join!(
            scim(&app, "POST", "/Users", &token, Some(user_body("x@example.com", "race"))),
            scim(&app, "POST", "/Users", &token, Some(user_body("y@example.com", "race"))),
        );
        let mut statuses = [left.0, right.0];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::CREATED, StatusCode::CONFLICT], "{left:?} {right:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn active_false_khoa_va_thu_hoi_phien_ngay() {
    TestDatabase::run(async |test| {
        let (app, services, admin, token) = setup(test).await;
        // Nhân viên có sẵn (tạo trước khi bật SCIM): POST cùng email thì nhận tài khoản đó.
        let (status, body) =
            scim(&app, "POST", "/Users", &token, Some(user_body("ada@example.com", "ada"))).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = body["id"].as_str().unwrap().to_owned();
        let ada = services
            .users
            .find_by_email(&services.tenant, "ada@example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ada.id.to_string(), id, "không tạo tài khoản thứ hai");
        let access = bearer(&app, "ada@example.com", PASSWORD).await;
        let refresh = {
            let body = json!({"email": "ada@example.com", "password": PASSWORD});
            let (_, _, login) = call(&app, "POST", "/login", None, Some(body)).await;
            login["data"]["refresh_token"].as_str().unwrap().to_owned()
        };
        // Ada giữ một role (đọc người dùng) để tạo được API key.
        let role = json!({"name": "reader", "permissions": ["auth.users.read"]});
        let (_, _, role) = call(&app, "POST", "/roles", Some(&admin), Some(role)).await;
        let roles = json!({"role_ids": [role["data"]["id"]]});
        let (status, _, _) =
            call(&app, "PUT", &format!("/users/{id}/roles"), Some(&admin), Some(roles)).await;
        assert_eq!(status, StatusCode::OK);
        let key_body =
            json!({"name": "ci", "permissions": ["auth.users.read"], "current_password": PASSWORD});
        let (status, _, key) = call(&app, "POST", "/api-keys", Some(&access), Some(key_body)).await;
        assert_eq!(status, StatusCode::CREATED, "{key}");
        let key = format!("Bearer {}", key["data"]["key"].as_str().unwrap());
        assert_eq!(call(&app, "GET", "/me", Some(&key), None).await.0, StatusCode::OK);

        // Cách Entra ID gọi: `Replace`, giá trị chuỗi "False".
        let patch = json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{"op": "Replace", "path": "active", "value": "False"}],
        });
        let (status, body) =
            scim(&app, "PATCH", &format!("/Users/{id}"), &token, Some(patch)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["active"], false);
        // Ngay lập tức: access token, refresh token, API key và mật khẩu đều không vào được.
        assert_eq!(call(&app, "GET", "/me", Some(&access), None).await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(&app, "GET", "/me", Some(&key), None).await.0, StatusCode::UNAUTHORIZED);
        let (status, _, _) =
            call(&app, "POST", "/refresh", None, Some(json!({"refresh_token": refresh}))).await;
        assert!(status.is_client_error(), "{status}");
        let login = json!({"email": "ada@example.com", "password": PASSWORD});
        assert!(call(&app, "POST", "/login", None, Some(login.clone())).await.0.is_client_error());
        // Event ghi bởi `scim`.
        let kinds = audit_kinds(&services).await;
        assert!(kinds.contains(&("auth.user_updated".to_owned(), "scim".to_owned())), "{kinds:?}");
        assert!(
            kinds.contains(&("auth.api_key_revoked".to_owned(), "scim".to_owned())),
            "{kinds:?}"
        );

        // Cách Okta bật lại: không `path`, `value` là object.
        let patch = json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{"op": "replace", "value": {"active": true}}],
        });
        let (status, body) =
            scim(&app, "PATCH", &format!("/Users/{id}"), &token, Some(patch)).await;
        assert_eq!((status, &body["active"]), (StatusCode::OK, &json!(true)), "{body}");
        assert_eq!(call(&app, "POST", "/login", None, Some(login)).await.0, StatusCode::OK);
        // API key đã thu hồi thì không sống lại.
        assert_eq!(call(&app, "GET", "/me", Some(&key), None).await.0, StatusCode::UNAUTHORIZED);

        // Khoá admin cuối cùng bị từ chối, không đổi gì.
        let (_, boss) =
            scim(&app, "POST", "/Users", &token, Some(user_body("boss@example.com", "boss"))).await;
        let boss = boss["id"].as_str().unwrap().to_owned();
        let mut body = user_body("boss@example.com", "boss");
        body["active"] = json!(false);
        let (status, refused) =
            scim(&app, "PUT", &format!("/Users/{boss}"), &token, Some(body)).await;
        assert_scim_error(status, &refused, StatusCode::CONFLICT, None);
        let (_, boss) = scim(&app, "GET", &format!("/Users/{boss}"), &token, None).await;
        assert_eq!(boss["active"], true);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn patch_theo_rfc_7644() {
    TestDatabase::run(async |test| {
        let (app, _, _, token) = setup(test).await;
        let (_, created) = scim(&app, "POST", "/Users", &token, Some(user_body("grace@example.com", "g-1"))).await;
        let id = created["id"].as_str().unwrap().to_owned();
        let path = format!("/Users/{id}");
        let patch = |operations: Value| {
            json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": operations})
        };
        let body = patch(json!([
            {"op": "replace", "path": "emails[type eq \"work\"].value", "value": "grace.hopper@example.com"},
            {"op": "add", "path": "name.givenName", "value": "Amazing Grace"},
            {"op": "replace", "path": "displayName", "value": "Rear Admiral Hopper"},
            {"op": "remove", "path": "externalId"},
            {"op": "add", "path": "urn:ietf:params:scim:schemas:extension:enterprise:2.0:User:department", "value": "Navy"},
        ]));
        let (status, body) = scim(&app, "PATCH", &path, &token, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["emails"][0]["value"], "grace.hopper@example.com");
        assert_eq!(body["name"]["givenName"], "Amazing Grace");
        assert_eq!(body["displayName"], "Rear Admiral Hopper");
        assert!(body.get("externalId").is_none(), "{body}");
        // Okta đổi userName qua `replace` không path.
        let body = patch(json!([{"op": "replace", "value": {"userName": "ghopper@example.com", "externalId": "g-2"}}]));
        let (status, body) = scim(&app, "PATCH", &path, &token, Some(body)).await;
        assert_eq!((status, body["userName"].as_str(), body["externalId"].as_str()), (StatusCode::OK, Some("ghopper@example.com"), Some("g-2")));

        for (operations, scim_type) in [
            (json!([{"op": "replace", "path": "id", "value": "x"}]), "mutability"),
            (json!([{"op": "remove"}]), "noTarget"),
            (json!([{"op": "copy", "path": "active"}]), "invalidSyntax"),
            (json!([{"op": "replace", "path": "active", "value": "maybe"}]), "invalidValue"),
            (json!([{"op": "replace", "path": "emails[type eq", "value": "x"}]), "invalidPath"),
        ] {
            let (status, body) = scim(&app, "PATCH", &path, &token, Some(patch(operations))).await;
            assert_scim_error(status, &body, StatusCode::BAD_REQUEST, Some(scim_type));
        }
        let wrong = json!({"schemas": ["urn:x"], "Operations": []});
        let (status, body) = scim(&app, "PATCH", &path, &token, Some(wrong)).await;
        assert_scim_error(status, &body, StatusCode::BAD_REQUEST, Some("invalidSyntax"));

        // DELETE: tài khoản bị xoá theo quy trình sẵn có.
        let (status, _) = scim(&app, "DELETE", &path, &token, None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) = scim(&app, "GET", &path, &token, None).await;
        assert_scim_error(status, &body, StatusCode::NOT_FOUND, None);
        let (status, body) = scim(&app, "DELETE", &path, &token, None).await;
        assert_scim_error(status, &body, StatusCode::NOT_FOUND, None);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_scim_khong_dung_duoc_cho_api_thuong() {
    TestDatabase::run(async |test| {
        let (app, _, admin, token) = setup(test).await;
        let scim_bearer = format!("Bearer {token}");
        // Token SCIM không mở được API thường.
        let (status, _, body) = call(&app, "GET", "/users", Some(&scim_bearer), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        // Access token / API key không mở được endpoint SCIM.
        let (status, body) =
            scim(&app, "GET", "/Users", admin.trim_start_matches("Bearer "), None).await;
        assert_scim_error(status, &body, StatusCode::UNAUTHORIZED, None);
        let (status, body) = scim(&app, "GET", "/Users", "axscim1_garbage", None).await;
        assert_scim_error(status, &body, StatusCode::UNAUTHORIZED, None);

        // Quản trị token: liệt kê không lộ secret, thu hồi thì bị từ chối ngay.
        let (status, _, list) = call(&app, "GET", "/scim/tokens", Some(&admin), None).await;
        assert_eq!(status, StatusCode::OK);
        let listed = list["data"].as_array().unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].get("token_secret").is_none());
        assert!(!list.to_string().contains(&token));
        let id = listed[0]["id"].as_str().unwrap();
        assert_eq!(scim(&app, "GET", "/Users", &token, None).await.0, StatusCode::OK);
        let (status, _, _) =
            call(&app, "DELETE", &format!("/scim/tokens/{id}"), Some(&admin), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(scim(&app, "GET", "/Users", &token, None).await.0, StatusCode::UNAUTHORIZED);

        // Người không có `auth.scim.manage` không tạo được token.
        let ada = bearer(&app, "ada@example.com", PASSWORD).await;
        let (status, _, body) =
            call(&app, "POST", "/scim/tokens", Some(&ada), Some(json!({"name": "x"}))).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("permission_denied"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn token_khach_a_khong_dong_bo_khach_b() {
    TestDatabase::run(async |test| {
        let services = crate::testing::services(test).await;
        let mailer: std::sync::Arc<dyn autoxide_kernel::mail::Mailer> = services.mails.clone();
        let app = TestApp::builder(test)
            .tenant("alpha")
            .tenant("beta")
            .provide::<dyn autoxide_kernel::mail::Mailer>(mailer)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let mut tokens = Vec::new();
        for slug in ["alpha", "beta"] {
            let tenant = app.tenant(slug).await.unwrap();
            let email = format!("boss@{slug}.example");
            let input = NewUser {
                email: email.clone(),
                name: email.clone(),
                password: Some(PASSWORD.to_owned()),
            };
            services.users.create_with_roles(&tenant, Actor::system(), input, true).await.unwrap();
            let response = app
                .post("/api/v1/auth/login")
                .header("autoxide-tenant", slug)
                .json(json!({"email": email, "password": PASSWORD}))
                .send()
                .await
                .unwrap();
            let admin =
                format!("Bearer {}", response.json()["data"]["access_token"].as_str().unwrap());
            tokens.push(scim_token(&app, &admin, Some(slug)).await);
        }
        let (alpha, beta) = (&tokens[0], &tokens[1]);
        let (status, created) =
            scim(&app, "POST", "/Users", alpha, Some(user_body("eve@example.com", "eve"))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().unwrap();
        // Khách beta không thấy, không sửa, không xoá được User của khách alpha.
        let (_, list) = scim(&app, "GET", "/Users", beta, None).await;
        assert_eq!(list["totalResults"], 0);
        assert_eq!(
            scim(&app, "GET", &format!("/Users/{id}"), beta, None).await.0,
            StatusCode::NOT_FOUND
        );
        let mut body = user_body("eve@example.com", "eve");
        body["active"] = json!(false);
        assert_eq!(
            scim(&app, "PUT", &format!("/Users/{id}"), beta, Some(body)).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            scim(&app, "DELETE", &format!("/Users/{id}"), beta, None).await.0,
            StatusCode::NOT_FOUND
        );
        // Cùng email ở khách beta là một người khác (khách khác, tài khoản khác).
        let (status, other) =
            scim(&app, "POST", "/Users", beta, Some(user_body("eve@example.com", "eve"))).await;
        assert_eq!(status, StatusCode::CREATED, "{other}");
        assert_ne!(other["id"], created["id"]);
        // Header khách khác không đổi được khách của token.
        let response = app
            .get(&format!("{BASE}/Users"))
            .header("authorization", &format!("Bearer {alpha}"))
            .header("autoxide-tenant", "beta")
            .send()
            .await
            .unwrap();
        assert_eq!(response.json()["Resources"][0]["id"], created["id"]);
        assert_eq!(response.json()["totalResults"], 1);
    })
    .await
    .unwrap();
}

/// (tên event, `actor_kind`) đã ghi vào `auth_events`.
async fn audit_kinds(services: &Services) -> Vec<(String, String)> {
    use sea_orm::ConnectionTrait;
    let backend = services.database.write().get_database_backend();
    let rows = services
        .database
        .write()
        .query_all_raw(sea_orm::Statement::from_string(
            backend,
            "SELECT name, actor_kind FROM auth_events ORDER BY id",
        ))
        .await
        .unwrap();
    rows.iter()
        .map(|row| (row.try_get("", "name").unwrap(), row.try_get("", "actor_kind").unwrap()))
        .collect()
}

/// Mã hoá phần trăm cho query (đủ cho bộ lọc trong test).
pub(crate) fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}
