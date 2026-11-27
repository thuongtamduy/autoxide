//! Sàn chặn mở bán hàng của người bán chưa định danh (LG-3, D-20261127-seller-kyc-listing): `catalog` hỏi
//! `KycGate` của `compliance` khi khách là sàn (`kernel.business_model = marketplace`). Chạy với module
//! `compliance` thật (quy tắc `kyc.required_activities` có ngày hiệu lực, nhà cung cấp định danh giả), qua cả
//! REST và MCP; `KycGate` giả chỉ dùng để chứng minh khách không phải sàn không bị hỏi.

use std::sync::Arc;
use std::time::Duration;

use autoxide_compliance::api::{Activity, KycGate, KycSession, KycStatus, Subject};
use autoxide_kernel::access::Scope;
use autoxide_kernel::error::Error;
use autoxide_kernel::identity::{IdentityVerifier, PartyType};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, OrgId, TenantId, root_of};
use autoxide_testkit::{FakeIdentityVerifier, TestApp, TestDatabase, tool};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Tree, app_around, app_with, call, create_product};

/// Đặt `kernel.business_model` của khách mặc định (trước lần đọc cài đặt đầu tiên: có bộ nhớ đệm).
async fn business_model(app: &TestApp, value: &str) {
    use sea_orm::ConnectionTrait as _;
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

async fn publish(app: &TestApp, id: &str) -> (StatusCode, Value) {
    let body = Some(json!({"status": "active"}));
    call(app, "POST", &format!("/products/{id}/status"), "boss", body).await
}

/// App có `compliance` thật (đăng ký trước `catalog`) và nhà cung cấp định danh giả; `kyc` là nhân viên sàn
/// được mở và nộp hồ sơ định danh của mọi nút.
async fn compliance_app(
    test: &TestDatabase,
    model: &str,
) -> (TestApp, Tree, Arc<FakeIdentityVerifier>) {
    let verifier = Arc::new(FakeIdentityVerifier::new());
    let provided: Arc<dyn IdentityVerifier> = verifier.clone();
    let grants = [("compliance.kyc.read", Scope::Tenant), ("compliance.kyc.submit", Scope::Tenant)];
    let (app, tree) = app_around(
        test,
        |builder| {
            builder
                .member("kyc", 9, root_of(DEFAULT_TENANT), &grants)
                .provide(provided)
                .module(autoxide_compliance::module())
        },
        |builder| builder,
    )
    .await;
    business_model(&app, model).await;
    (app, tree, verifier)
}

/// Định danh nút `org` qua nhà cung cấp giả (đăng ký kinh doanh) tới khi hồ sơ `verified`.
async fn verify_seller(app: &TestApp, verifier: &FakeIdentityVerifier, org: OrgId) {
    let compliance = |method: &str, uri: &str, body: Option<Value>| {
        let method = axum::http::Method::from_bytes(method.as_bytes()).unwrap();
        let mut request = app.request(method, &format!("/api/v1/compliance{uri}")).token("kyc");
        if let Some(body) = body {
            request = request.json(body);
        }
        request
    };
    let body = json!({"org_id": org.as_uuid(), "party_type": "organization"});
    let opened = compliance("POST", "/kyc/profiles", Some(body)).send().await.unwrap();
    assert!(opened.status().is_success(), "{:?}", opened.body());
    let profile = opened.json()["data"]["id"].as_str().unwrap().to_owned();
    let body = json!({"method": "business_registry"});
    let uri = format!("/kyc/profiles/{profile}/sessions");
    let session = compliance("POST", &uri, Some(body)).send().await.unwrap();
    assert_eq!(session.status(), StatusCode::OK, "{:?}", session.body());
    let attempt: Uuid = session.json()["data"]["attempt_id"].as_str().unwrap().parse().unwrap();
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
    let read = compliance("GET", &format!("/kyc/profiles/{profile}"), None).send().await.unwrap();
    assert_eq!(read.json()["data"]["status"], "verified");
}

/// Sàn: sản phẩm của người bán chưa định danh không mở bán được — qua REST lẫn tool MCP — với mã
/// `compliance.kyc_required` (403), sản phẩm vẫn `draft`; định danh xong thì mở bán được. Hàng của chính sàn
/// (nút gốc) không cần định danh người bán.
#[tokio::test]
async fn san_chan_hang_cua_nguoi_ban_chua_dinh_danh() {
    TestDatabase::run(async |test| {
        let (app, tree, verifier) = compliance_app(test, "marketplace").await;
        let (seller, _) = create_product(&app, "boss", tree.a, "K-1", "Hàng người bán").await;

        let (status, problem) = publish(&app, &seller).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem["code"], "compliance.kyc_required");

        let client = app.mcp("boss").await.unwrap();
        let input = json!({"id": seller, "status": "active"});
        let refused =
            client.call_tool(tool("catalog.set_product_status", input.clone())).await.unwrap();
        assert_eq!(refused.structured_content.unwrap()["code"], "compliance.kyc_required");

        let (_, still) = call(&app, "GET", &format!("/products/{seller}"), "boss", None).await;
        assert_eq!(still["data"]["status"], "draft");

        let root = root_of(DEFAULT_TENANT);
        let (own, _) = create_product(&app, "boss", root, "K-2", "Hàng của sàn").await;
        let (status, sold) = publish(&app, &own).await;
        assert_eq!(status, StatusCode::OK, "{sold}");

        verify_seller(&app, &verifier, tree.a).await;
        let listed = client.call_tool(tool("catalog.set_product_status", input)).await.unwrap();
        assert_eq!(listed.is_error, Some(false), "{listed:?}");
        assert_eq!(listed.structured_content.unwrap()["data"]["status"], "active");
        client.cancel().await.unwrap();

        // Người bán khác (nút `b`) vẫn chưa định danh: định danh của `a` không mở cho `b`.
        let (other, _) = create_product(&app, "boss", tree.b, "K-3", "Hàng người bán khác").await;
        let (status, problem) = publish(&app, &other).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
    })
    .await
    .unwrap();
}

/// Không phải sàn (`multi_store`: cửa hàng tự bán hàng của mình) dù có `compliance`: không qua bước định danh.
#[tokio::test]
async fn khong_phai_san_mo_ban_khong_can_dinh_danh() {
    TestDatabase::run(async |test| {
        let (app, tree, _) = compliance_app(test, "multi_store").await;
        let (product, _) = create_product(&app, "boss", tree.a, "K-1", "Hàng cửa hàng").await;
        let (status, sold) = publish(&app, &product).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
    })
    .await
    .unwrap();
}

/// `KycGate` giả ghi lại mọi lần bị hỏi.
#[derive(Default)]
struct Recorder(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl KycGate for Recorder {
    async fn require_verified(
        &self,
        _tenant: TenantId,
        subject: Subject,
        activity: Activity,
    ) -> Result<(), Error> {
        let (_, id) = subject.columns();
        self.0.lock().unwrap().push(format!("{id}:{}", activity.as_str()));
        Ok(())
    }

    async fn status(&self, _tenant: TenantId, _subject: Subject) -> Result<KycStatus, Error> {
        Err(Error::internal("not used in this test"))
    }

    async fn start(
        &self,
        _tenant: TenantId,
        _subject: Subject,
        _party: PartyType,
        _return_url: Option<String>,
    ) -> Result<KycSession, Error> {
        Err(Error::internal("not used in this test"))
    }
}

/// Chỉ sàn mới hỏi, và hỏi đúng người bán (nút sở hữu sản phẩm) cho việc `sell`; khách chưa đặt mô hình
/// kinh doanh, hay đặt mô hình khác, không bị hỏi.
#[tokio::test]
async fn chi_san_hoi_dinh_danh_dung_nguoi_ban() {
    for (model, asked) in [(None, false), (Some("independent"), false), (Some("marketplace"), true)]
    {
        TestDatabase::run(async |test| {
            let recorder = Arc::new(Recorder::default());
            let gate: Arc<dyn KycGate> = recorder.clone();
            let (app, tree) = app_with(test, |builder| builder.provide(gate)).await;
            if let Some(model) = model {
                business_model(&app, model).await;
            }
            let (product, _) = create_product(&app, "boss", tree.a, "K-1", "Hàng").await;
            let (status, sold) = publish(&app, &product).await;
            assert_eq!(status, StatusCode::OK, "{sold}");
            let expected =
                if asked { vec![format!("{}:sell", tree.a.as_uuid())] } else { Vec::new() };
            assert_eq!(*recorder.0.lock().unwrap(), expected, "{model:?}");
        })
        .await
        .unwrap();
    }
}

/// Sàn không cài `compliance` (tự ghép module): không có gì để hỏi, mở bán được (máy chủ ghi log cảnh báo).
#[tokio::test]
async fn san_khong_co_compliance_khong_chan() {
    TestDatabase::run(async |test| {
        let (app, tree) = super::app(test).await;
        business_model(&app, "marketplace").await;
        let (product, _) = create_product(&app, "boss", tree.a, "K-1", "Hàng").await;
        let (status, sold) = publish(&app, &product).await;
        assert_eq!(status, StatusCode::OK, "{sold}");
    })
    .await
    .unwrap();
}
