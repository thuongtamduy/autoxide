//! Hồ sơ Bộ Công Thương và trang bắt buộc trên chương trình thật (bước 37, D-357, D-358, D-359): hồ
//! sơ pháp lý → website → danh sách kiểm báo thiếu trang → `content` tạo trang nháp từ mẫu (biến từ
//! hồ sơ pháp lý qua `LegalProfiles`, không đọc bảng của `compliance`) → đăng → gắn trang → mục đạt →
//! bộ hồ sơ có URL trang → ghi trạng thái "đã thông báo" kèm logo → logo công khai theo tên miền.
//! Chạy trên cả SQLite và Postgres.

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-moit-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_CONTENT_LOCALES", "en,vi")
        .user(
            "admin",
            1,
            &[
                "compliance.legal.read",
                "compliance.legal.manage",
                "content.read",
                "content.write",
                "content.publish",
            ],
        );
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1{uri}")).token("admin");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

fn status_of(checklist: &Value, key: &str) -> String {
    checklist["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["key"] == key)
        .map(|item| item["status"].as_str().unwrap().to_owned())
        .unwrap()
}

#[tokio::test]
async fn trang_tu_mau_gan_website_roi_logo_cong_khai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, profile) = call(
            &app,
            Method::POST,
            "/compliance/legal-profiles",
            Some(json!({"entity_type": "organization", "legal_name": "Cong ty TNHH Vi Du",
                "tax_code": "0101234567", "email": "lienhe@shop.example", "phone": "024 1234 5678"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{profile}");
        let (status, site) = call(
            &app,
            Method::POST,
            "/compliance/websites",
            Some(json!({"domain": "shop.example", "kind": "sales"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{site}");
        let id = site["data"]["id"].as_str().unwrap().to_owned();
        let (_, checklist) =
            call(&app, Method::GET, &format!("/compliance/websites/{id}/checklist"), None).await;
        assert_eq!(status_of(&checklist, "page.return_policy"), "missing");
        assert_eq!(status_of(&checklist, "owner.tax_code"), "met");

        // Mẫu trang: danh sách có đủ hai ngôn ngữ; tạo nháp lấy tên chủ sở hữu từ hồ sơ pháp lý.
        let (status, templates) = call(&app, Method::GET, "/content/templates", None).await;
        assert_eq!(status, StatusCode::OK, "{templates}");
        assert!(templates["data"].as_array().unwrap().len() >= 11);
        let (status, problem) = call(
            &app,
            Method::POST,
            "/content/templates/return_policy/drafts",
            Some(json!({"locale": "vi"})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["code"], "content.template_variables_missing", "{problem}");
        let (status, entry) = call(
            &app,
            Method::POST,
            "/content/templates/return_policy/drafts",
            Some(json!({"locale": "vi", "variables": {"policy": {"return_days": 7}}})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{entry}");
        assert!(entry["data"]["body"].to_string().contains("Cong ty TNHH Vi Du"), "{entry}");
        let entry_id = entry["data"]["id"].as_str().unwrap().to_owned();

        // Gắn bản nháp: chưa đạt; đăng rồi thì đạt.
        let link = format!("/compliance/websites/{id}/pages/return_policy");
        let (status, _) =
            call(&app, Method::PUT, &link, Some(json!({"entry_id": entry_id}))).await;
        assert_eq!(status, StatusCode::OK);
        let (_, checklist) =
            call(&app, Method::GET, &format!("/compliance/websites/{id}/checklist"), None).await;
        assert_eq!(status_of(&checklist, "page.return_policy"), "missing");
        let published = app
            .post(&format!("/api/v1/content/entries/{entry_id}/publish"))
            .token("admin")
            .header("if-match", "*")
            .send()
            .await
            .unwrap();
        assert!(published.status().is_success(), "{}", published.text());
        let (_, checklist) =
            call(&app, Method::GET, &format!("/compliance/websites/{id}/checklist"), None).await;
        assert_eq!(status_of(&checklist, "page.return_policy"), "met");
        let (_, dossier) =
            call(&app, Method::GET, &format!("/compliance/websites/{id}/dossier"), None).await;
        let pages = dossier["data"]["form"]["pages"].to_string();
        assert!(pages.contains("https://shop.example/"), "{pages}");

        // Đã thông báo: logo công khai theo tên miền.
        let snippet = "<a href='http://online.gov.vn/Home/WebDetails/1'><img src='http://online.gov.vn/Content/EndUser/LogoCCDVSaleNoti/logoSaleNoti.png'/></a>";
        let (status, done) = call(
            &app,
            Method::POST,
            &format!("/compliance/websites/{id}/status"),
            Some(json!({"status": "notified", "snippet": snippet})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{done}");
        let badge =
            app.get("/api/v1/compliance/public/badge").host("shop.example").send().await.unwrap();
        assert_eq!(badge.status(), StatusCode::OK, "{}", badge.text());
        assert_eq!(badge.json()["data"]["color"], "blue");
        let owner = app.get("/api/v1/compliance/public/owner").send().await.unwrap();
        assert_eq!(owner.json()["data"]["tax_code"], "0101234567");
    })
    .await
    .unwrap();
}
