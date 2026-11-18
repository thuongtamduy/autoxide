//! Tự động hoá (bước 58) với module thật: đăng nhập thật của `auth` (quyền HIỆN TẠI của người chạy
//! luật qua `Authenticator::principal_of`), hành động gọi tool MCP thật (`auth.update_user`), và audit
//! ghi thay đổi với đường vào `automation` + ID lần chạy. Chạy trên cả SQLite và Postgres.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use serde_json::{Value, json};

async fn app(test: &TestDatabase) -> (TestApp, String, String) {
    let dir = std::env::temp_dir().join(format!("autoxide-automations-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    let app = builder.build().await.unwrap();
    let account = NewAccount {
        email: "admin@example.com".to_owned(),
        name: "Admin".to_owned(),
        password: Some("correct horse battery staple".to_owned()),
        admin: true,
    };
    let admin = app
        .api::<dyn Accounts>()
        .unwrap()
        .create(&TenantContext::default_on(app.database().clone()), account)
        .await
        .unwrap();
    let login = app
        .post("/api/v1/auth/login")
        .json(json!({"email": "admin@example.com", "password": "correct horse battery staple"}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200, "{}", login.text());
    let token = login.json()["data"]["access_token"].as_str().unwrap().to_owned();
    (app, token, admin.id.to_string())
}

async fn settle(app: &TestApp) {
    let dispatcher = app.modules().dispatcher().unwrap();
    let worker = app.modules().worker(4).unwrap();
    for _ in 0..10 {
        let delivered = dispatcher.run_once().await.unwrap();
        let ran = worker.run_once().await.unwrap();
        if delivered == 0 && ran == 0 {
            break;
        }
    }
}

#[tokio::test]
async fn luat_goi_tool_that_voi_quyen_nguoi_tao_va_audit_thay_lan_chay() {
    TestDatabase::run(async |test| {
        let (app, token, admin_id) = app(test).await;
        // Danh mục có tool thật của module khác, không có tool của chính `automations`.
        let catalog = app.get("/api/v1/automations/catalog").token(&token).send().await.unwrap();
        assert_eq!(catalog.status(), 200, "{}", catalog.text());
        let tools: Vec<String> = catalog.json()["data"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["tool"].as_str().unwrap().to_owned())
            .collect();
        assert!(tools.contains(&"auth.update_user".to_owned()));
        assert!(!tools.iter().any(|tool| tool.starts_with("automations.")));
        // Luật gọi tay: đổi tên người dùng theo dữ liệu vào.
        let rule = json!({
            "name": "Rename",
            "trigger": {"type": "manual"},
            "actions": [{"tool": "auth.update_user", "arguments": {"id": "{{ event.user_id }}", "name": "{{ event.name }}"}}]
        });
        let created = app.post("/api/v1/automations/rules").token(&token).json(rule).send().await.unwrap();
        assert_eq!(created.status(), 201, "{}", created.text());
        let rule_id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let input = json!({"input": {"user_id": admin_id, "name": "Renamed by a rule"}});
        let run = app
            .post(&format!("/api/v1/automations/rules/{rule_id}/run"))
            .token(&token)
            .json(input)
            .send()
            .await
            .unwrap();
        assert_eq!(run.status(), 202, "{}", run.text());
        let run_id = run.json()["data"]["id"].as_str().unwrap().to_owned();
        settle(&app).await;
        let read = app.get(&format!("/api/v1/automations/runs/{run_id}")).token(&token).send().await.unwrap();
        let body: Value = read.json();
        assert_eq!(body["data"]["status"], "succeeded", "{body}");
        assert_eq!(body["data"]["steps"][0]["result"]["name"], "Renamed by a rule", "{body}");
        // Audit: thay đổi do lần chạy gây ra mang đường vào `automation` và ID lần chạy.
        let entries = app.get("/api/v1/audit/entries?limit=200").token(&token).send().await.unwrap();
        assert_eq!(entries.status(), 200, "{}", entries.text());
        let text = entries.text();
        assert!(text.contains("\"automation\""), "{text}");
        assert!(text.contains(&run_id), "{text}");
    })
    .await
    .unwrap();
}
