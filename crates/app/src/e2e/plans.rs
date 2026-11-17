//! Gói, hạn mức và quyền dùng trên module thật (bước 35 phần D): hạn mức số nút tổ chức, dung lượng
//! file, đếm request API; sổ module ghi tính năng và hạn mức các module khai. Chạy trên cả SQLite và
//! Postgres.

use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::body::Body;
use axum::http::{Method, StatusCode};
use sea_orm::ConnectionTrait;
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
            &[
                "plans.plans.manage",
                "plans.subscriptions.manage",
                "plans.subscriptions.read",
                "kernel.modules.read",
                "kernel.tenants.read",
                "kernel.tenants.manage",
                "kernel.tenants.delete",
            ],
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

/// Hạn mức theo loại nút (A-3.12): gói đặt `organizations.units.store` = 1 bên cạnh mức chung; loại
/// không đặt mức riêng chỉ tính vào mức chung; lưu trữ trả lại chỗ của loại; đối soát đếm lại theo
/// loại; danh sách quyền dùng có dòng của loại.
#[tokio::test]
async fn han_muc_theo_loai_nut() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        put_on(
            &app,
            "typed",
            &[],
            json!({"organizations.units": 3, "organizations.units.store": 1}),
        )
        .await;
        for code in ["store", "warehouse"] {
            let body = json!({"code": code, "name": code});
            let uri = "/api/v1/organizations/unit-types";
            let (status, _) = call(&app, Method::POST, uri, "owner", Some(body)).await;
            assert_eq!(status, StatusCode::CREATED);
        }
        let root = app.tenant("acme").await.unwrap().id().as_uuid();
        let create = async |kind: &str, name: &str| {
            let body = json!({"parent_id": root, "unit_type": kind, "name": name});
            call(&app, Method::POST, "/api/v1/organizations/units", "owner", Some(body)).await
        };
        let (status, store) = create("store", "S1").await;
        assert_eq!(status, StatusCode::CREATED, "{store}");
        let (status, full) = create("store", "S2").await;
        assert_eq!(
            (status, full["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("limit_exceeded"))
        );
        assert!(full.to_string().contains("organizations.units.store"), "{full}");
        // Loại không có mức riêng: chỉ tính vào mức chung (3).
        assert_eq!(create("warehouse", "W1").await.0, StatusCode::CREATED);
        // Lưu trữ cửa hàng đầu: trả lại cả chỗ chung lẫn chỗ của loại.
        let id = store["data"]["id"].as_str().unwrap();
        let uri = format!("/api/v1/organizations/units/{id}/status");
        let (status, _) =
            call(&app, Method::POST, &uri, "owner", Some(json!({"status": "archived"}))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(create("store", "S2").await.0, StatusCode::CREATED);
        assert_eq!(create("warehouse", "W2").await.0, StatusCode::CREATED);
        // Đầy mức chung (3 nút không lưu trữ) dù loại `warehouse` không có mức riêng.
        let (status, full) = create("warehouse", "W3").await;
        assert_eq!(
            (status, full["code"].as_str()),
            (StatusCode::FORBIDDEN, Some("limit_exceeded"))
        );

        let (_, entitlements) =
            call(&app, Method::GET, "/api/v1/plans/entitlements", "owner", None).await;
        let limits = entitlements["data"]["limits"].as_array().unwrap();
        let find = |key: &str| limits.iter().find(|limit| limit["key"] == key).cloned();
        let store = find("organizations.units.store").unwrap();
        assert_eq!((store["max"].clone(), store["used"].clone()), (json!(1), json!(1)));
        let warehouse = find("organizations.units.warehouse").unwrap();
        assert_eq!((warehouse["max"].clone(), warehouse["used"].clone()), (json!(null), json!(2)));

        // Số dùng bị lệch (tiến trình chết giữa chừng): job đối soát đếm lại, cả theo loại.
        let statement = sea_orm::Statement::from_string(
            app.database().write().get_database_backend(),
            "UPDATE plans_usage SET used = 9 WHERE limit_key LIKE 'organizations.units%'",
        );
        app.database().write().execute_raw(statement).await.unwrap();
        drain(&app).await;
        let (_, entitlements) =
            call(&app, Method::GET, "/api/v1/plans/entitlements", "owner", None).await;
        let limits = entitlements["data"]["limits"].as_array().unwrap();
        let used =
            |key: &str| limits.iter().find(|limit| limit["key"] == key).unwrap()["used"].clone();
        assert_eq!(used("organizations.units"), json!(3), "{entitlements}");
        assert_eq!(used("organizations.units.store"), json!(1));
        assert_eq!(used("organizations.units.warehouse"), json!(2));

        // Sổ module nói hạn mức chia theo loại nút.
        let (_, module) =
            call(&app, Method::GET, "/api/v1/kernel/modules/organizations", "ops", None).await;
        assert_eq!(module["data"]["limits"][0]["parts"], "unit type", "{module}");
    })
    .await
    .unwrap();
}

/// Đếm dòng của khách `tenant` trong bảng `table` (database chung của test).
async fn rows_of(app: &TestApp, table: &str, tenant: uuid::Uuid) -> i64 {
    let database = app.database().write();
    let backend = database.get_database_backend();
    let row = database
        .query_one_raw(sea_orm::Statement::from_sql_and_values(
            backend,
            format!("SELECT COUNT(*) AS n FROM {table} WHERE tenant_id = $1"),
            [tenant.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

/// Xoá hẳn một khách (D-337): chỉ khi đã tạm khoá, xác nhận bằng slug; dữ liệu và file của khách
/// mất, khách khác không bị đụng, sổ khách giữ dòng `deleted`, request tới khách nhận 404.
#[tokio::test]
async fn xoa_han_mot_khach_chi_khi_da_tam_khoa() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = app.tenant("acme").await.unwrap().id().as_uuid();
        let beta = app.tenant("beta").await.unwrap().id().as_uuid();
        let body = json!({"code": "store", "name": "Store"});
        let (status, _) =
            call(&app, Method::POST, "/api/v1/organizations/unit-types", "owner", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        // Một file của khách: bản xuất dữ liệu.
        let base = format!("/api/v1/kernel/tenants/{acme}/exports");
        let (status, _) = call(&app, Method::POST, &base, "owner", None).await;
        assert_eq!(status, StatusCode::CREATED);
        drain(&app).await;
        let types = "organizations_unit_type";
        assert!(rows_of(&app, types, acme).await > 0);
        let beta_before = rows_of(&app, types, beta).await;

        let erase = format!("/api/v1/kernel/tenants/{acme}/erase");
        let body = json!({"confirm": "acme", "reason": "Asked to close the account"});
        let (status, refused) = call(&app, Method::POST, &erase, "ops", Some(body.clone())).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenant_not_suspended"))
        );
        let suspend = json!({"status": "suspended", "reason": "Closing"});
        let uri = format!("/api/v1/kernel/tenants/{acme}");
        let (status, _) = call(&app, Method::PATCH, &uri, "ops", Some(suspend)).await;
        assert_eq!(status, StatusCode::OK);
        let wrong = json!({"confirm": "beta", "reason": "Asked to close the account"});
        let (status, refused) = call(&app, Method::POST, &erase, "ops", Some(wrong)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["errors"][0]["field"], "confirm");
        let (status, refused) = call(&app, Method::POST, &erase, "owner", Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

        let (status, erased) = call(&app, Method::POST, &erase, "ops", Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{erased}");
        assert!(erased["data"]["rows"].as_u64().unwrap() > 0);
        assert!(erased["data"]["files"].as_u64().unwrap() >= 1, "{erased}");
        assert_eq!(rows_of(&app, types, acme).await, 0);
        assert_eq!(rows_of(&app, types, beta).await, beta_before);
        assert_eq!(rows_of(&app, "kernel_tenant_exports", acme).await, 0);

        let (_, row) = call(&app, Method::GET, &uri, "ops", None).await;
        assert_eq!(row["data"]["status"], "deleted");
        let (status, gone) =
            call(&app, Method::GET, "/api/v1/organizations/unit-types", "owner", None).await;
        assert_eq!(
            (status, gone["code"].as_str()),
            (StatusCode::NOT_FOUND, Some("tenant_not_found"))
        );
        let (status, _) = call(&app, Method::POST, &erase, "ops", Some(body)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let platform = json!({"confirm": "default", "reason": "No"});
        let (status, refused) =
            call(&app, Method::POST, "/api/v1/kernel/tenants/default/erase", "ops", Some(platform))
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    })
    .await
    .unwrap();
}

/// Cấp phát trọn gói qua `api::Provisioning` (đường của `autoxide tenant create --plan`) với module
/// thật: khách mới có thuê bao, quản trị viên được mời với role `admin` của khách mới (D-338).
#[tokio::test]
async fn cap_phat_khach_moi_voi_module_that() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let body = json!({"code": "starter", "name": "Starter", "status": "active"});
        let (status, plan) =
            call(&app, Method::POST, "/api/v1/plans/plans", "ops", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{plan}");
        let plan_id: uuid::Uuid = plan["data"]["id"].as_str().unwrap().parse().unwrap();
        let provisioning = app.api::<dyn autoxide_plans::api::Provisioning>().unwrap();
        let request = autoxide_plans::api::ProvisionRequest::new(
            "gamma",
            "Gamma Ltd",
            plan_id,
            "owner@gamma.example",
        );
        let done = provisioning.provision(request).await.unwrap();
        assert_eq!((done.step.as_str(), done.status.as_str()), ("done", "done"));
        let tenant = done.tenant_id.unwrap();
        let uri = format!("/api/v1/plans/tenants/{tenant}/subscription");
        let (status, sub) = call(&app, Method::GET, &uri, "ops", None).await;
        assert_eq!(status, StatusCode::OK, "{sub}");
        assert_eq!(sub["data"]["plan_code"], "starter");
        assert_eq!(rows_of(&app, "auth_invitation", tenant).await, 1);
    })
    .await
    .unwrap();
}
