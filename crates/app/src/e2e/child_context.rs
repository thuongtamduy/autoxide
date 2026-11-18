//! Cha thao tác trên con xuyên database (bước 38, mục 2): người của khách cha vào khách con có
//! database RIÊNG bằng uỷ quyền theo cây (bước 34) — cùng API, chỉ thêm header `Autoxide-Tenant`.
//! Thay đổi ghi ở database của con (audit của con thấy "vào bằng uỷ quyền" kèm ID uỷ quyền), việc
//! cấp / thu hồi ghi ở database của cha. Chạy trên cả SQLite và Postgres.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse battery staple";

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-child-{}", uuid::Uuid::now_v7()));
    TestApp::builder(test)
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .tenant("parent")
        .child_tenant("child", "parent", true)
        .module(autoxide_mail::module())
        .module(autoxide_auth::module())
        .module(autoxide_audit::module())
        .build()
        .await
        .unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    token: Option<&str>,
    tenant: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request =
        app.request(method, &format!("/api/v1/auth{path}")).header("autoxide-tenant", tenant);
    if let Some(token) = token {
        request = request.token(token);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    (status, if response.body().is_empty() { Value::Null } else { response.json() })
}

async fn sign_in(app: &TestApp, email: &str) -> String {
    let body = json!({"email": email, "password": PASSWORD});
    let (status, answer) = call(app, Method::POST, "/login", None, "parent", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["data"]["access_token"].as_str().unwrap().to_owned()
}

/// Dòng audit `(action, details)` của khách `tenant` trên database của nó.
async fn audit(tenant: &TenantContext) -> Vec<(String, String)> {
    let read = tenant.database().read();
    let rows = read
        .query_all_raw(Statement::from_sql_and_values(
            read.get_database_backend(),
            "SELECT action, details FROM audit_entry WHERE tenant_id = $1 ORDER BY id",
            [tenant.id().as_uuid().into()],
        ))
        .await
        .unwrap();
    rows.iter()
        .map(|row| {
            let action: String = row.try_get("", "action").unwrap();
            let details: Option<String> = row.try_get("", "details").unwrap();
            (action, details.unwrap_or_default())
        })
        .collect()
}

#[tokio::test]
async fn parent_acts_in_child_on_other_database_and_audit_recorded_on_both_sides() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let parent = app.tenant("parent").await.unwrap();
        let child = app.tenant("child").await.unwrap();
        assert_ne!(parent.placement(), child.placement(), "the child has its own database");
        let accounts = app.api::<dyn Accounts>().unwrap();
        for (email, admin) in [("boss@parent.example", true), ("agent@parent.example", false)] {
            let account = NewAccount {
                email: email.to_owned(),
                name: email.to_owned(),
                password: Some(PASSWORD.to_owned()),
                admin,
            };
            accounts.create(&parent, account).await.unwrap();
        }
        let boss = sign_in(&app, "boss@parent.example").await;
        let role = json!({"name": "chain_ops", "permissions": ["auth.roles.read", "auth.roles.manage"]});
        let (status, role) =
            call(&app, Method::POST, "/roles", Some(&boss), "parent", Some(role)).await;
        assert_eq!(status, StatusCode::CREATED, "{role}");
        let role_id = role["data"]["id"].as_str().unwrap().to_owned();
        let (_, users) = call(&app, Method::GET, "/users", Some(&boss), "parent", None).await;
        let agent_id = users["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["email"] == "agent@parent.example")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let roles = json!({"role_ids": [role_id]});
        let path = format!("/users/{agent_id}/roles");
        let (status, _) = call(&app, Method::PUT, &path, Some(&boss), "parent", Some(roles)).await;
        assert_eq!(status, StatusCode::OK);
        let agent = sign_in(&app, "agent@parent.example").await;
        let new_role = |name: &str| json!({"name": name, "permissions": ["auth.roles.read"]});

        // Chưa uỷ quyền: không vào được khách con.
        let (status, _) =
            call(&app, Method::POST, "/roles", Some(&agent), "child", Some(new_role("x"))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let grant = json!({"child_tenant_id": child.id().as_uuid(), "role_id": role_id, "reason": "Run the shop's roles"});
        let (status, granted) =
            call(&app, Method::POST, "/delegations", Some(&boss), "parent", Some(grant)).await;
        assert_eq!(status, StatusCode::CREATED, "{granted}");
        let delegation = granted["data"]["id"].as_str().unwrap().to_owned();

        // Cùng API, ghi vào database riêng của khách con.
        let (status, created) =
            call(&app, Method::POST, "/roles", Some(&agent), "child", Some(new_role("cashier")))
                .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let (_, listed) = call(&app, Method::GET, "/roles", Some(&agent), "child", None).await;
        assert!(listed["data"].as_array().unwrap().iter().any(|role| role["name"] == "cashier"));
        let (_, at_parent) = call(&app, Method::GET, "/roles", Some(&boss), "parent", None).await;
        assert!(!at_parent["data"].as_array().unwrap().iter().any(|role| role["name"] == "cashier"));

        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        // Audit của con (database của con): việc làm mang đường vào "uỷ quyền" và ID uỷ quyền.
        let child_audit = audit(&child).await;
        let (_, details) =
            child_audit.iter().find(|(action, _)| action == "auth.role_created").unwrap();
        assert!(details.contains("\"delegation\"") && details.contains(&delegation), "{details}");
        // Audit của cha (database của cha): ai được vào con nào.
        let parent_audit = audit(&parent).await;
        assert!(parent_audit.iter().any(|(action, _)| action == "auth.delegation_granted"));
        assert!(!parent_audit.iter().any(|(action, details)| action == "auth.role_created"
            && details.contains("cashier")));

        // revoked_delegation_blocks_child_access: thu hồi có hiệu lực ngay request kế.
        let path = format!("/delegations/{delegation}");
        let (status, _) = call(&app, Method::DELETE, &path, Some(&boss), "parent", None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) =
            call(&app, Method::POST, "/roles", Some(&agent), "child", Some(new_role("late")))
                .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        while dispatcher.run_once().await.unwrap() > 0 {}
        assert!(audit(&parent).await.iter().any(|(action, _)| action == "auth.delegation_revoked"));
    })
    .await
    .unwrap();
}
