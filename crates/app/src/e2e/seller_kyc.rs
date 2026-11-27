//! Sàn chặn mở bán hàng của người bán chưa định danh trên chương trình thật (LG-3,
//! D-20261127-seller-kyc-listing): `catalog` hỏi `KycGate` của `compliance` (quy tắc `kyc.required_activities`
//! có ngày hiệu lực, nhà cung cấp định danh giả) khi khách là sàn (`kernel.business_model = marketplace`), qua cả
//! REST và tool MCP. Chạy trên cả SQLite và Postgres.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Scope;
use autoxide_kernel::identity::IdentityVerifier;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, OrgTree, root_of};
use autoxide_testkit::{FakeIdentityVerifier, TestApp, TestDatabase, tool};
use axum::http::{Method, StatusCode};
use sea_orm::ConnectionTrait as _;
use serde_json::{Value, json};
use uuid::Uuid;

const GRANTS: [&str; 5] = [
    "catalog.products.read",
    "catalog.products.write",
    "catalog.products.publish",
    "compliance.kyc.read",
    "compliance.kyc.submit",
];

/// Hai người bán (`a`, `b`) dưới gốc của khách mặc định; `a` có một cửa hàng con `store` (sàn → người bán →
/// cửa hàng, D-185).
struct Sellers {
    a: OrgId,
    b: OrgId,
    store: OrgId,
}

async fn app(test: &TestDatabase, model: &str) -> (TestApp, Sellers, Arc<FakeIdentityVerifier>) {
    let dir = std::env::temp_dir().join(format!("autoxide-seller-kyc-{}", Uuid::now_v7()));
    let verifier = Arc::new(FakeIdentityVerifier::new());
    let provided: Arc<dyn IdentityVerifier> = verifier.clone();
    let grants = GRANTS.map(|name| (name, Scope::Tenant));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .provide(provided)
        .member("boss", 1, root_of(DEFAULT_TENANT), &grants);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    let app = builder.build().await.unwrap();
    let org = || OrgId::from_uuid(Uuid::now_v7());
    let sellers = Sellers { a: org(), b: org(), store: org() };
    let write = app.database().begin_write().await.unwrap();
    let root = OrgTree::insert_root(&write, DEFAULT_TENANT).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, sellers.a, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, sellers.b, root, 8).await.unwrap();
    OrgTree::insert(&write, DEFAULT_TENANT, sellers.store, sellers.a, 8).await.unwrap();
    write.commit().await.unwrap();
    business_model(&app, model).await;
    (app, sellers, verifier)
}

/// Đặt `kernel.business_model` của khách mặc định (trước lần đọc cài đặt đầu tiên: có bộ nhớ đệm).
async fn business_model(app: &TestApp, value: &str) {
    let write = app.database().write();
    let backend = write.get_database_backend();
    let sql = "INSERT INTO kernel_tenant_settings (tenant_id, name, value, secret, updated_by, updated_at) \
               VALUES ($1, 'kernel.business_model', $2, FALSE, NULL, $3)";
    let values: Vec<sea_orm::Value> = vec![
        DEFAULT_TENANT.as_uuid().into(),
        value.into(),
        autoxide_kernel::time::Timestamp::now().into(),
    ];
    write.execute_raw(autoxide_kernel::database::statement(backend, sql, values)).await.unwrap();
}

async fn call(app: &TestApp, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1{uri}")).token("boss");
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Tạo sản phẩm (dịch vụ: không cần tồn kho) ở nút `org`; trả ID.
async fn product(app: &TestApp, org: OrgId, sku: &str) -> String {
    let body = json!({
        "org_id": org.as_uuid(), "kind": "service", "tax_category": "vat.standard", "locale": "vi",
        "text": {"name": format!("Dịch vụ {sku}")}, "variant": {"sku": sku},
    });
    let (status, created) = call(app, "POST", "/catalog/products", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["data"]["id"].as_str().unwrap().to_owned()
}

async fn publish(app: &TestApp, id: &str) -> (StatusCode, Value) {
    let body = Some(json!({"status": "active"}));
    call(app, "POST", &format!("/catalog/products/{id}/status"), body).await
}

/// Định danh nút `org` qua nhà cung cấp giả (đăng ký kinh doanh) tới khi hồ sơ `verified`.
async fn verify_seller(app: &TestApp, verifier: &FakeIdentityVerifier, org: OrgId) {
    let body = json!({"org_id": org.as_uuid(), "party_type": "organization"});
    let (status, opened) = call(app, "POST", "/compliance/kyc/profiles", Some(body)).await;
    assert!(status.is_success(), "{opened}");
    let profile = opened["data"]["id"].as_str().unwrap().to_owned();
    let uri = format!("/compliance/kyc/profiles/{profile}/sessions");
    let body = json!({"method": "business_registry"});
    let (status, session) = call(app, "POST", &uri, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    let attempt: Uuid = session["data"]["attempt_id"].as_str().unwrap().parse().unwrap();
    let reference = FakeIdentityVerifier::reference(attempt);
    verifier.complete(&reference, true, "CONG TY TNHH NGUOI BAN", "0101234567");
    let (headers, body) = FakeIdentityVerifier::callback(&reference);
    let mut request = app.post("/api/v1/compliance/kyc/callbacks/fake").bytes(body);
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    let answer = request.header("content-type", "application/json").send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::NO_CONTENT);
    app.modules().worker(8).unwrap().blocked_delay(Duration::ZERO).run_once().await.unwrap();
    let (_, read) = call(app, "GET", &format!("/compliance/kyc/profiles/{profile}"), None).await;
    assert_eq!(read["data"]["status"], "verified", "{read}");
}

/// Sàn: sản phẩm của người bán chưa định danh không mở bán được — qua REST lẫn tool MCP — với mã
/// `compliance.kyc_required` (403), sản phẩm vẫn `draft`; hàng của chính sàn (nút gốc) không cần; định danh
/// xong thì mở bán được, kể cả hàng ở cửa hàng con của người bán (định danh ở nút cha là đủ); người bán khác
/// vẫn bị chặn.
#[tokio::test]
async fn san_chan_hang_cua_nguoi_ban_chua_dinh_danh() {
    TestDatabase::run(async |test| {
        let (app, sellers, verifier) = app(test, "marketplace").await;
        let seller = product(&app, sellers.a, "K-1").await;

        let (status, problem) = publish(&app, &seller).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem["code"], "compliance.kyc_required");

        let client = app.mcp("boss").await.unwrap();
        let input = json!({"id": seller, "status": "active"});
        let refused =
            client.call_tool(tool("catalog.set_product_status", input.clone())).await.unwrap();
        assert_eq!(refused.structured_content.unwrap()["code"], "compliance.kyc_required");

        let in_store = product(&app, sellers.store, "K-4").await;
        let (status, problem) = publish(&app, &in_store).await;
        assert_eq!(problem["code"], "compliance.kyc_required", "{status}");

        let (_, still) = call(&app, "GET", &format!("/catalog/products/{seller}"), None).await;
        assert_eq!(still["data"]["status"], "draft");

        let own = product(&app, root_of(DEFAULT_TENANT), "K-2").await;
        let (status, sold) = publish(&app, &own).await;
        assert_eq!(status, StatusCode::OK, "{sold}");

        verify_seller(&app, &verifier, sellers.a).await;
        let listed = client.call_tool(tool("catalog.set_product_status", input)).await.unwrap();
        assert_eq!(listed.is_error, Some(false), "{listed:?}");
        assert_eq!(listed.structured_content.unwrap()["data"]["status"], "active");
        client.cancel().await.unwrap();

        let (status, sold) = publish(&app, &in_store).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "hàng ở cửa hàng con của người bán đã định danh: {sold}"
        );

        let other = product(&app, sellers.b, "K-3").await;
        let (status, problem) = publish(&app, &other).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    })
    .await
    .unwrap();
}

/// Không phải sàn (`multi_store`: cửa hàng tự bán hàng của mình): không qua bước định danh người bán.
#[tokio::test]
async fn khong_phai_san_mo_ban_khong_can_dinh_danh() {
    TestDatabase::run(async |test| {
        let (app, sellers, _) = app(test, "multi_store").await;
        let id = product(&app, sellers.a, "K-1").await;
        let (status, sold) = publish(&app, &id).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
    })
    .await
    .unwrap();
}
