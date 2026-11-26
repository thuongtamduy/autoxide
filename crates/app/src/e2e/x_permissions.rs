//! PT-13: `x-permissions` của mỗi operation khớp chỗ service kiểm. Với mỗi permission `P` mà một module khai, một
//! tài khoản chỉ có `P` gọi mọi operation của cùng module mà `P` KHÔNG nằm trong `x-permissions`; service phải từ
//! chối (403 `permission_denied`, hay 404 khi operation khai `x-hides-existence`). Trả gì khác (2xx, 404, 422…) nghĩa
//! là service đã cho `P` qua bước kiểm quyền mà tài liệu không nêu — client và agent đọc tài liệu sẽ hiểu sai.

use std::collections::BTreeMap;

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

use super::access::{probe, send};

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
                let probe = probe(&document, method, path, operation);
                for (name, token) in &tokens {
                    if module_of(name) != module || listed.contains(&name.as_str()) {
                        continue;
                    }
                    let (status, body) = send(&app, &probe, Some(token)).await;
                    let refused = status == 403 && body["code"] == "permission_denied"
                        || hides && status == 404;
                    if !refused {
                        findings.push(format!(
                            "{} {path}: `{name}` (not in x-permissions {listed:?}) answered {status} {}",
                            method.to_uppercase(),
                            body["code"]
                        ));
                    }
                }
            }
        }
        assert!(findings.is_empty(), "{} finding(s):\n{}", findings.len(), findings.join("\n"));
    })
    .await
    .unwrap();
}

/// PT-14: người không có permission nào nhận 403 `permission_denied` TRƯỚC mọi bước kiểm khác — kể cả khi request
/// thiếu `If-Match`, thân sai và ID không có: không học được endpoint cần `ETag`, trường nào sai hay bản ghi nào tồn
/// tại. (Operation khai `x-hides-existence` thì 404 cũng được.)
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
