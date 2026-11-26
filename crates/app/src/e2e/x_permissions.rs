//! PT-13: `x-permissions` của mỗi operation khớp chỗ service kiểm. App dựng KHÔNG có lớp permission của kernel
//! (`without_permission_gate`) để thấy service tự nhận gì. Với mỗi permission `P` mà một module khai, một tài khoản
//! chỉ có `P` gọi mọi operation của cùng module mà `P` KHÔNG nằm trong `x-permissions`: service phải từ chối (403
//! `permission_denied`). Trả gì khác nghĩa là service nhận `P` mà tài
//! liệu không nêu — khi bật lớp permission, người giữ `P` bị chặn oan. 422 `validation_failed` (service kiểm dữ liệu
//! trước quyền) và 404 của operation khai `x-hides-existence` không kết luận được nên không tính; operation nào không có
//! lượt nào kết luận được thì test đỏ (bài dò không được qua mà không thử gì).
//!
//! Chiều ngược lại (permission ĐƯỢC khai mà service vẫn từ chối) không kiểm ở đây: vài operation cần cùng lúc hai
//! permission, `x-permissions` ("một trong các permission là đủ") không diễn đạt được (D-20261126-permission-gate).

use std::collections::BTreeMap;

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

use axum::http::Method;

use super::access::{Probe, probe, send};

const PASSWORD: &str = "correct horse battery staple";

async fn account(app: &TestApp, email: &str, admin: bool) -> String {
    let account = NewAccount {
        email: email.to_owned(),
        name: "U".to_owned(),
        password: Some(PASSWORD.to_owned()),
        admin,
    };
    app.api::<dyn Accounts>()
        .unwrap()
        .create(&TenantContext::default_on(app.database().clone()), account)
        .await
        .unwrap();
    let login = app
        .post("/api/v1/auth/login")
        .json(json!({"email": email, "password": PASSWORD}))
        .send()
        .await
        .unwrap();
    login.json()["data"]["access_token"].as_str().unwrap().to_owned()
}

fn module_of(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

#[tokio::test]
async fn x_permissions_khop_cho_service_kiem() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-xperm-{}", uuid::Uuid::now_v7()));
        let mut builder = TestApp::builder(test)
            .without_permission_gate()
            .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
            .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let document = serde_json::to_value(app.openapi()).unwrap();
        let admin = account(&app, "root@example.com", true).await;
        // Mỗi permission một role và một tài khoản chỉ có role đó.
        let mut tokens: BTreeMap<String, String> = BTreeMap::new();
        let permissions: Vec<String> =
            app.modules().permissions().iter().map(|permission| permission.name().to_owned()).collect();
        for (index, name) in permissions.iter().enumerate() {
            let role = app
                .post("/api/v1/auth/roles")
                .token(&admin)
                .json(json!({"name": format!("only-{index}"), "permissions": [name]}))
                .send()
                .await
                .unwrap();
            let role = role.json()["data"]["id"].as_str().unwrap().to_owned();
            let email = format!("p{index}@example.com");
            let token = account(&app, &email, false).await;
            let me = app.get("/api/v1/auth/me").token(&token).send().await.unwrap().json();
            let user = me["data"]["id"].as_str().unwrap().to_owned();
            let assigned = app
                .request(axum::http::Method::PUT, &format!("/api/v1/auth/users/{user}/roles"))
                .token(&admin)
                .json(json!({"role_ids": [role]}))
                .send()
                .await
                .unwrap();
            assert!(assigned.status().is_success(), "{}", assigned.json());
            tokens.insert(name.clone(), token);
        }

        let mut findings = Vec::new();
        let mut untested = Vec::new();
        for (path, item) in document["paths"].as_object().unwrap() {
            for (method, operation) in item.as_object().unwrap() {
                if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                    continue;
                }
                let Some(listed) = operation.get("x-permissions").and_then(Value::as_array) else {
                    continue;
                };
                let listed: Vec<&str> = listed.iter().filter_map(Value::as_str).collect();
                let Some(first) = listed.first() else { continue };
                let module = module_of(first);
                let hides = operation.get("x-hides-existence") == Some(&json!(true));
                let mut probe = probe(&document, method, path, operation);
                // `limit=0` (cố ý sai, cho bài thứ tự PT-14) làm service không có lớp permission trả 422 trước khi
                // kiểm quyền: bỏ đi để danh sách cũng được dò.
                probe.uri = probe.uri.replace("?limit=0&", "?").replace("?limit=0", "").replace("&limit=0", "");
                // Lượt có kết luận: service từ chối bằng 403 `permission_denied` (đã tới bước kiểm quyền), hay nhận.
                // 422 `validation_failed` (kiểm dữ liệu trước quyền) và 404 của operation ẩn tồn tại (ID ngẫu nhiên
                // không có, kiểm quyền hay không cũng 404) không nói gì về quyền.
                let (mut tried, mut conclusive) = (0, 0);
                for (name, token) in &tokens {
                    if module_of(name) != module || listed.contains(&name.as_str()) {
                        continue;
                    }
                    tried += 1;
                    let (status, body) = send(&app, &probe, Some(token)).await;
                    let refused = status == 403 && body["code"] == "permission_denied";
                    let inconclusive = status == 422 && body["code"] == "validation_failed"
                        || hides && status == 404;
                    if refused {
                        conclusive += 1;
                    } else if !inconclusive {
                        conclusive += 1;
                        findings.push(format!(
                            "{} {path}: `{name}` (not in x-permissions {listed:?}) answered {status} {}",
                            method.to_uppercase(),
                            body["code"]
                        ));
                    }
                }
                let key = format!("{} {path}", method.to_uppercase());
                if tried > 0 && conclusive == 0 {
                    untested.push(key);
                }
            }
        }
        // Operation không có lượt nào kết luận được thì bài dò không nói gì về nó: đỏ để mẫu request được sửa.
        assert!(
            untested.is_empty(),
            "{} operation(s) never reached a permission check:\n{}",
            untested.len(),
            untested.join("\n")
        );
        assert!(findings.is_empty(), "{} finding(s):\n{}", findings.len(), findings.join("\n"));
    })
    .await
    .unwrap();
}

/// PT-14: người không có permission nào nhận 403 `permission_denied` TRƯỚC mọi bước kiểm khác — kể cả khi request
/// thiếu `If-Match`, thân sai và ID không có: không học được endpoint cần `ETag`, trường nào sai hay bản ghi nào tồn
/// tại. (Operation khai `x-hides-existence` thì 404 cũng được.) `HEAD` của operation `GET` cũng vậy; người chưa đăng
/// nhập nhận 401 trước mọi bước kiểm dữ liệu.
#[tokio::test]
async fn kiem_quyen_truoc_du_lieu_va_ban_ghi() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-order-{}", uuid::Uuid::now_v7()));
        let mut builder = TestApp::builder(test)
            .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
            .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let document = serde_json::to_value(app.openapi()).unwrap();
        let mut token = account(&app, "nobody@example.com", false).await;
        let mut findings = Vec::new();
        for (path, item) in document["paths"].as_object().unwrap() {
            for (method, operation) in item.as_object().unwrap() {
                if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                    continue;
                }
                let listed = operation.get("x-permissions").and_then(Value::as_array);
                if listed.is_none_or(Vec::is_empty) {
                    continue;
                }
                let mut probe = probe(&document, method, path, operation);
                // Dữ liệu sai: không header bắt buộc (`If-Match`…), thân không khớp schema.
                probe.headers.clear();
                if let Some((media, bytes)) = probe.body.as_mut()
                    && media.ends_with("json")
                {
                    *bytes = br#"{"__invalid": true}"#.to_vec();
                }
                let (mut status, mut body) = send(&app, &probe, Some(&token)).await;
                if body["code"] == "auth.session_ended" {
                    token = account_login(&app, "nobody@example.com").await;
                    (status, body) = send(&app, &probe, Some(&token)).await;
                }
                let hides = operation.get("x-hides-existence") == Some(&json!(true));
                if !(status == 403 && body["code"] == "permission_denied" || hides && status == 404)
                {
                    findings.push(format!(
                        "{} {path}: answered {status} {}",
                        method.to_uppercase(),
                        body["code"]
                    ));
                }
                // `HEAD` chạy handler `get` của axum: cũng phải bị chặn (thân trả lời rỗng, chỉ xem mã).
                if method == "get" {
                    let head = Probe { method: Method::HEAD, ..probe.clone() };
                    let (status, _) = send(&app, &head, Some(&token)).await;
                    if !(status == 403 || hides && status == 404) {
                        findings.push(format!("HEAD {path}: answered {status}"));
                    }
                }
                // Không đăng nhập: 401 trước mọi bước kiểm dữ liệu.
                let (status, body) = send(&app, &probe, None).await;
                if status != 401 {
                    findings.push(format!(
                        "{} {path} without a token: answered {status} {}",
                        method.to_uppercase(),
                        body["code"]
                    ));
                }
            }
        }
        assert!(findings.is_empty(), "{} finding(s):\n{}", findings.len(), findings.join("\n"));
    })
    .await
    .unwrap();
}

async fn account_login(app: &TestApp, email: &str) -> String {
    let login = app
        .post("/api/v1/auth/login")
        .json(json!({"email": email, "password": PASSWORD}))
        .send()
        .await
        .unwrap();
    login.json()["data"]["access_token"].as_str().unwrap().to_owned()
}
