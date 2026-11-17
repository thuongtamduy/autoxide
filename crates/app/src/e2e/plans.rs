//! Gói, hạn mức và quyền dùng trên module thật (bước 35 phần D): hạn mức số nút tổ chức, dung lượng
//! file, đếm request API; sổ module ghi tính năng và hạn mức các module khai. Chạy trên cả SQLite và
//! Postgres.

use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::body::Body;
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

const OWNER: [&str; 7] = [
    "kernel.data.export",
    "organizations.units.read",
    "organizations.units.write",
    "organizations.units.manage",
    "organizations.types.manage",
    "files.upload",
    "forms.read",
];

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-plans-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .tenant("acme")
        .user(
            "ops",
            1,
            &["plans.plans.manage", "plans.subscriptions.manage", "kernel.modules.read"],
        )
        .user_in("acme", "owner", 2, &OWNER)
        .tenant("beta")
        .user_in("beta", "stranger", 3, &["kernel.data.export"]);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, uri).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Đặt khách `acme` vào gói bật `modules` với hạn mức `limits`.
async fn put_on(app: &TestApp, code: &str, modules: &[&str], limits: Value) -> TenantId {
    let body = json!({"code": code, "name": code, "status": "active", "modules": modules, "limits": limits});
    let (status, plan) = call(app, Method::POST, "/api/v1/plans/plans", "ops", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{plan}");
    let acme = app.tenant("acme").await.unwrap().id();
    let body = json!({"plan_id": plan["data"]["id"], "trial": false});
    let uri = format!("/api/v1/plans/tenants/{acme}/subscription");
    let (status, sub) = call(app, Method::PUT, &uri, "ops", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{sub}");
    acme
}

#[tokio::test]
async fn han_muc_so_nut_to_chuc_tinh_ca_luu_tru() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        put_on(&app, "small", &[], json!({"organizations.units": 1})).await;
        let body = json!({"code": "store", "name": "Store"});
        let (status, _) =
            call(&app, Method::POST, "/api/v1/organizations/unit-types", "owner", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        let root = app.tenant("acme").await.unwrap().id().as_uuid();
        let unit = |name: &str| json!({"parent_id": root, "unit_type": "store", "name": name});
        let (status, first) =
            call(&app, Method::POST, "/api/v1/organizations/units", "owner", Some(unit("A"))).await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        let (status, full) =
            call(&app, Method::POST, "/api/v1/organizations/units", "owner", Some(unit("B"))).await;
        assert_eq!(
            (status, full["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("limit_exceeded")),
            "{full}"
        );
        // Lưu trữ nút đầu: trả lại chỗ; tạo nút khác được.
        let id = first["data"]["id"].as_str().unwrap();
        let archive = json!({"status": "archived"});
        let uri = format!("/api/v1/organizations/units/{id}/status");
        let (status, _) = call(&app, Method::POST, &uri, "owner", Some(archive)).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) =
            call(&app, Method::POST, "/api/v1/organizations/units", "owner", Some(unit("B"))).await;
        assert_eq!(status, StatusCode::CREATED);
        // Bỏ lưu trữ nút đầu khi đã đầy: bị chặn.
        let (status, body) =
            call(&app, Method::POST, &uri, "owner", Some(json!({"status": "active"}))).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("limit_exceeded")),
            "{body}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn han_muc_dung_luong_file_tra_lai_khi_xoa() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        put_on(&app, "tiny", &[], json!({"files.storage_bytes": 10})).await;
        let upload = |bytes: &'static [u8]| {
            app.request(Method::POST, "/api/v1/files?name=a.txt")
                .token("owner")
                .header("content-type", "text/plain")
                .body(Body::from(bytes))
        };
        let first = upload(b"12345678").send().await.unwrap();
        assert_eq!(first.status(), StatusCode::CREATED, "{}", first.text());
        let full = upload(b"12345").send().await.unwrap();
        assert_eq!(
            (full.status(), full.json()["code"].clone()),
            (StatusCode::FORBIDDEN, json!("limit_exceeded"))
        );
        let id = first.json()["data"]["id"].as_str().unwrap().to_owned();
        let (status, _) =
            call(&app, Method::DELETE, &format!("/api/v1/files/{id}"), "owner", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let again = upload(b"12345").send().await.unwrap();
        assert_eq!(again.status(), StatusCode::CREATED, "{}", again.text());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dem_request_api_va_het_quota_tra_429() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = put_on(&app, "metered", &["forms"], json!({"kernel.api_requests": 2})).await;
        for _ in 0..3 {
            let (status, body) = call(&app, Method::GET, "/api/v1/forms", "owner", None).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        // Ghi gộp: 3 > 2 → hết quota; request sau của module bị gói chi phối trả 429.
        let entitlements = app.modules().entitlements();
        entitlements.flush_usage("test-node", 1).await;
        assert!(entitlements.request_quota_wait(acme).is_some());
        let (status, body) = call(&app, Method::GET, "/api/v1/forms", "owner", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::TOO_MANY_REQUESTS, Some("quota_exhausted"))
        );
        // Module nền vẫn chạy: xem quyền dùng được.
        let (status, _) =
            call(&app, Method::GET, "/api/v1/plans/entitlements", "owner", None).await;
        assert_eq!(status, StatusCode::OK);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_module_ghi_tinh_nang_han_muc_va_module_can() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (_, auth) = call(&app, Method::GET, "/api/v1/kernel/modules/auth", "ops", None).await;
        assert_eq!(auth["data"]["features"][0]["key"], "auth.api_keys", "{auth}");
        assert_eq!(auth["data"]["limits"][0]["key"], "auth.members");
        assert_eq!(auth["data"]["always_enabled"], true);
        let (_, content) =
            call(&app, Method::GET, "/api/v1/kernel/modules/content", "ops", None).await;
        assert_eq!(content["data"]["always_enabled"], false);
        assert_eq!(content["data"]["features"][0]["key"], "content.ai_suggestions");
        // `content` cần `media` (trait `Media`): gói bật `content` phải bật cả `media`.
        let requires = content["data"]["requires"].as_array().unwrap();
        assert!(requires.contains(&json!("media")), "{requires:?}");
        let body = json!({"code": "bad", "name": "Bad", "modules": ["content"]});
        let (status, refused) =
            call(&app, Method::POST, "/api/v1/plans/plans", "ops", Some(body)).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::UNPROCESSABLE_ENTITY, Some("plans.module_dependency_missing"))
        );
        let (_, kernel) =
            call(&app, Method::GET, "/api/v1/kernel/modules/kernel", "ops", None).await;
        assert_eq!(kernel["data"]["limits"][0]["key"], "kernel.api_requests");
    })
    .await
    .unwrap();
}

/// Chạy worker tới khi hết job.
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap();
    for _ in 0..10 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

#[tokio::test]
async fn xuat_toan_bo_du_lieu_mot_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = app.tenant("acme").await.unwrap().id();
        // Dữ liệu của khách: một nút tổ chức.
        let body = json!({"code": "store", "name": "Store marker-export"});
        let (status, _) =
            call(&app, Method::POST, "/api/v1/organizations/unit-types", "owner", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        let base = format!("/api/v1/kernel/tenants/{acme}/exports");
        let (status, export) = call(&app, Method::POST, &base, "owner", None).await;
        assert_eq!(status, StatusCode::CREATED, "{export}");
        let id = export["data"]["id"].as_str().unwrap().to_owned();
        // Người của khách khác: như không có khách đó.
        let (status, body) = call(&app, Method::POST, &base, "stranger", None).await;
        assert_eq!(
            (status, body["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenant_not_found"))
        );
        drain(&app).await;
        let (_, got) = call(&app, Method::GET, &format!("{base}/{id}"), "owner", None).await;
        assert_eq!(got["data"]["status"], "ready", "{got}");
        let content = app.get(&format!("{base}/{id}/content")).token("owner").send().await.unwrap();
        assert_eq!(content.status(), StatusCode::OK);
        let text = content.text();
        let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(first["format"], "autoxide-tenant-export");
        assert!(text.contains("marker-export"), "dữ liệu của khách có trong bản xuất");
        assert!(!text.contains("password_hash"), "cột bí mật không có");
        // Link tải không cần đăng nhập; link mới thay link cũ.
        let (_, link) = call(&app, Method::POST, &format!("{base}/{id}/link"), "owner", None).await;
        let url = link["data"]["url"].as_str().unwrap().to_owned();
        let opened = app.get(&url).send().await.unwrap();
        assert_eq!(opened.status(), StatusCode::OK);
        assert_eq!(opened.text(), text);
        call(&app, Method::POST, &format!("{base}/{id}/link"), "owner", None).await;
        let stale = app.get(&url).send().await.unwrap();
        assert_eq!(
            (stale.status(), stale.json()["code"].clone()),
            (StatusCode::NOT_FOUND, json!("kernel.export_link_invalid"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn huy_thue_bao_tu_xin_xuat_du_lieu() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = put_on(&app, "basic", &[], json!({})).await;
        let uri = format!("/api/v1/plans/tenants/{acme}/subscription/cancel");
        let (status, body) = call(&app, Method::POST, &uri, "ops", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let exports = app.api::<autoxide_kernel::tenancy::TenantExports>().unwrap();
        let listed = exports.list(acme).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].reason, "subscription_canceled");
    })
    .await
    .unwrap();
}
