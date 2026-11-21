//! Sandbox cách ly (bước 38, D-232, D-20261119-step38-sandbox-isolation): đối tác luôn là bộ giả
//! (không tới bộ nối thật, ghi hộp thư giả), mail chỉ tới nhân viên vào được sandbox, hộp thư giả
//! đọc được qua REST (tool MCP `kernel.list_sandbox_messages` gọi cùng hàm); khách thật không bị ảnh hưởng. Chạy với khách gốc ở database chung và
//! database riêng, trên cả SQLite và Postgres.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::sync::Arc;
use std::time::Duration;

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::mail::{Mail, Mailer};
use autoxide_kernel::money::Money;
use autoxide_kernel::payment::{GatewayPaymentStatus, PaymentGateway, PaymentIntent};
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose, SmsSender};
use autoxide_kernel::tenancy::{TenantContext, TenantId};
use autoxide_testkit::{FakePaymentGateway, FakeSmsSender, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse battery staple";

struct Fakes {
    sms: Arc<FakeSmsSender>,
    gateway: Arc<FakePaymentGateway>,
    voice: Arc<autoxide_testkit::FakeVoiceProvider>,
}

async fn app(test: &TestDatabase, dedicated: bool) -> (TestApp, Fakes) {
    let dir = std::env::temp_dir().join(format!("autoxide-sandbox-iso-{}", uuid::Uuid::now_v7()));
    let fakes = Fakes {
        sms: Arc::new(FakeSmsSender::new()),
        gateway: Arc::new(FakePaymentGateway::new()),
        voice: Arc::new(autoxide_testkit::FakeVoiceProvider::default()),
    };
    let builder = TestApp::builder(test)
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string());
    let builder = if dedicated { builder.tenant_dedicated("shop") } else { builder.tenant("shop") };
    let app = builder
        .module(autoxide_mail::module())
        .module(autoxide_auth::module())
        .provide::<dyn SmsSender>(fakes.sms.clone())
        .contribute::<dyn PaymentGateway>(fakes.gateway.clone())
        .contribute::<dyn autoxide_kernel::voice::VoiceProvider>(fakes.voice.clone())
        .build()
        .await
        .unwrap();
    (app, fakes)
}

async fn call(
    app: &TestApp,
    method: Method,
    path: &str,
    token: &str,
    tenant: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request =
        app.request(method, &format!("/api/v1{path}")).header("autoxide-tenant", tenant);
    if !token.is_empty() {
        request = request.token(token);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let body = if response.body().is_empty() { Value::Null } else { response.json() };
    (status, body)
}

/// Chủ khách `shop` (role `admin`, email đã xác minh), đã đăng nhập; trả access token.
async fn owner(app: &TestApp, shop: &TenantContext) -> String {
    let email = "owner@shop.example";
    let account = NewAccount {
        email: email.to_owned(),
        name: "Owner".to_owned(),
        password: Some(PASSWORD.to_owned()),
        admin: true,
    };
    app.api::<dyn Accounts>().unwrap().create(shop, account).await.unwrap();
    let write = shop.database().write();
    write
        .execute_raw(Statement::from_sql_and_values(
            write.get_database_backend(),
            "UPDATE auth_user SET email_verified_at = $1 WHERE email = $2",
            [Timestamp::now().into(), email.into()],
        ))
        .await
        .unwrap();
    let body = json!({"email": email, "password": PASSWORD});
    let (status, answer) = call(app, Method::POST, "/auth/login", "", "shop", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    answer["data"]["access_token"].as_str().unwrap().to_owned()
}

async fn settle(app: &TestApp) {
    let worker = app.modules().worker(8).unwrap().blocked_delay(Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..5 {
        let ran = worker.run_once().await.unwrap();
        let delivered = dispatcher.run_once().await.unwrap();
        if ran == 0 && delivered == 0 {
            break;
        }
    }
}

fn sms(tenant: TenantId) -> SmsMessage {
    let to = PhoneNumber::parse("+84901234567", "84").unwrap();
    SmsMessage::new(tenant, to, "Your code is 123456", SmsPurpose::Otp, "otp-1")
}

async fn isolation(dedicated: bool) {
    TestDatabase::run(async |test| {
        let (app, fakes) = app(test, dedicated).await;
        let shop = app.tenant("shop").await.unwrap();
        let token = owner(&app, &shop).await;
        let path = format!("/kernel/tenants/{}/sandboxes", shop.id());
        let (status, created) =
            call(&app, Method::POST, &path, &token, "shop", Some(json!({"name": "Try"}))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        settle(&app).await;
        let sandbox =
            app.registry().tenant(TenantId::from_uuid(id.parse().unwrap())).await.unwrap();
        assert!(sandbox.is_sandbox());

        // doi_tac_trong_sandbox_luon_la_bo_gia: SMS không tới bộ nối, vào hộp thư giả.
        let sender = app.api::<dyn SmsSender>().unwrap();
        let receipt = sender.send(sms(sandbox.id())).await.unwrap();
        assert!(receipt.provider_reference.starts_with("SANDBOX-"), "{receipt:?}");
        assert!(fakes.sms.sent.lock().unwrap().is_empty(), "không gửi SMS thật");
        // Khách thật vẫn đi bộ nối.
        sender.send(sms(shop.id())).await.unwrap();
        assert_eq!(fakes.sms.sent.lock().unwrap().len(), 1);

        // Thanh toán: thành công giả, không tạo lượt thu ở cổng.
        let gateways = app.modules().contributions::<dyn PaymentGateway>();
        assert_eq!(gateways.len(), 1);
        let amount = Money::from_columns(250_000, "VND").unwrap();
        let expires = Timestamp::now() + SignedDuration::from_mins(15);
        let intent =
            PaymentIntent::new(uuid::Uuid::now_v7(), amount, "card", "Sandbox order", expires);
        let payment = gateways[0].create_payment(sandbox.id(), &intent).await.unwrap();
        assert!(payment.provider_ref.starts_with("SANDBOX-"));
        assert!(fakes.gateway.payments.lock().unwrap().is_empty(), "không gọi cổng thật");
        let state = gateways[0].payment_status(sandbox.id(), &payment.provider_ref).await.unwrap();
        assert_eq!(state, GatewayPaymentStatus::Succeeded { amount });
        gateways[0].create_payment(shop.id(), &intent).await.unwrap();
        assert_eq!(fakes.gateway.payments.lock().unwrap().len(), 1, "khách thật gọi cổng thật");

        // mail_sandbox_chi_toi_nhan_vien: chủ khách (vào được sandbox) nhận thật; người ngoài bị
        // giữ lại. Một mail tới người ngoài không chặn mail khác.
        let mailer = app.api::<dyn Mailer>().unwrap();
        let stranger = Mail::new("Buyer <buyer@example.com>", "Order shipped", "Hello");
        mailer.send(&sandbox, stranger).await.unwrap();
        let staff = Mail::new("owner@shop.example", "Sandbox ready", "Hello");
        mailer.send(&sandbox, staff).await.unwrap();

        // Hộp thư giả qua REST: ba thứ bị giữ, mới nhất trước; lọc theo kênh.
        let (status, held) =
            call(&app, Method::GET, "/kernel/sandbox/messages", &token, &id, None).await;
        assert_eq!(status, StatusCode::OK, "{held}");
        let items = held["data"].as_array().unwrap();
        let channels: Vec<&str> =
            items.iter().map(|item| item["channel"].as_str().unwrap()).collect();
        assert_eq!(channels, ["mail", "partner", "sms"], "{held}");
        assert_eq!(items[0]["reason"], "recipient_not_allowed");
        assert_eq!(items[0]["recipient"], "Buyer <buyer@example.com>");
        assert_eq!(items[1]["provider"], "fake");
        assert_eq!(items[1]["subject"], "payment.create");
        assert_eq!(items[1]["body"]["amount"]["minor"], 250_000);
        assert_eq!(items[2]["recipient"], "+84901234567");
        let (_, only) =
            call(&app, Method::GET, "/kernel/sandbox/messages?channel=sms", &token, &id, None)
                .await;
        assert_eq!(only["data"].as_array().unwrap().len(), 1);
        // Ở khách thật: chỉ có nghĩa trong sandbox.
        let (status, refused) =
            call(&app, Method::GET, "/kernel/sandbox/messages", &token, "shop", None).await;
        assert_eq!(
            (status, refused["code"].as_str()),
            (StatusCode::CONFLICT, Some("tenancy.sandbox_only"))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sandbox_cach_ly_database_chung() {
    isolation(false).await;
}

#[tokio::test]
async fn sandbox_cach_ly_database_rieng() {
    isolation(true).await;
}

/// Sandbox dùng quyền dùng của khách gốc (không thuê bao riêng), huy hiệu pháp lý là "bản thử" (không
/// bao giờ logo Bộ Công Thương), chủ website ghi `sandbox: true`. Mọi module thật.
#[tokio::test]
async fn sandbox_dung_quyen_cua_khach_goc_va_huy_hieu_ban_thu() {
    TestDatabase::run(async |test| {
        let dir =
            std::env::temp_dir().join(format!("autoxide-sandbox-all-{}", uuid::Uuid::now_v7()));
        let mut builder = TestApp::builder(test)
            .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
            .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
            .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
            .tenant("shop")
            .user("ops", 1, &["plans.plans.manage", "plans.subscriptions.manage"])
            .user_in("shop", "owner", 2, &["kernel.sandboxes.manage", "kernel.sandboxes.read"]);
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let shop = app.tenant("shop").await.unwrap();

        // Khách gốc trên gói chỉ bật `forms`, hạn mức sandbox 1.
        let body = json!({"code": "small", "name": "Small", "status": "active",
            "modules": ["forms"], "limits": {"kernel.sandboxes": 1}});
        let plan = app.post("/api/v1/plans/plans").token("ops").json(body).send().await.unwrap();
        assert_eq!(plan.status(), StatusCode::CREATED);
        let body = json!({"plan_id": plan.json()["data"]["id"], "trial": false});
        let uri = format!("/api/v1/plans/tenants/{}/subscription", shop.id());
        let subscribed = app.put(&uri).token("ops").json(body).send().await.unwrap();
        assert_eq!(subscribed.status(), StatusCode::OK);

        let path = format!("/api/v1/kernel/tenants/{}/sandboxes", shop.id());
        let created =
            app.post(&path).token("owner").json(json!({"name": "Try"})).send().await.unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let id: uuid::Uuid = created.json()["data"]["id"].as_str().unwrap().parse().unwrap();
        settle(&app).await;
        let sandbox = TenantId::from_uuid(id);

        let entitlements = app.modules().entitlements();
        let origin = entitlements.snapshot(shop.id()).await.unwrap();
        let copy = entitlements.snapshot(sandbox).await.unwrap();
        assert!(!copy.unrestricted, "sandbox không có thuê bao nhưng không thành không giới hạn");
        assert_eq!(copy.modules, origin.modules);
        assert_eq!(copy.limits, origin.limits);

        // Huy hiệu công khai của sandbox: nhãn "bản thử", không link / ảnh của Bộ.
        let badge = app
            .get("/api/v1/compliance/public/badge?locale=en")
            .header("autoxide-tenant", &id.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(badge.status(), StatusCode::OK);
        let badge: Value = badge.json();
        assert_eq!(badge["data"]["kind"], "sandbox", "{badge}");
        assert_eq!(badge["data"]["link_url"], "");
        assert_eq!(badge["data"]["image_url"], "");
        assert!(badge["data"]["alt"].as_str().unwrap().contains("Test copy"));
    })
    .await
    .unwrap();
}

/// Gọi điện (bước 61, D-232): sandbox không gọi nhà cung cấp thoại thật; khách thật vẫn gọi.
#[tokio::test]
async fn thoai_trong_sandbox_la_gia() {
    use autoxide_kernel::voice::{OutboundCall, VoiceProvider};
    TestDatabase::run(async |test| {
        let (app, fakes) = app(test, false).await;
        let shop = app.tenant("shop").await.unwrap();
        let token = owner(&app, &shop).await;
        let path = format!("/kernel/tenants/{}/sandboxes", shop.id());
        let (status, created) =
            call(&app, Method::POST, &path, &token, "shop", Some(json!({"name": "Try"}))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["data"]["id"].as_str().unwrap().to_owned();
        settle(&app).await;
        let sandbox =
            app.registry().tenant(TenantId::from_uuid(id.parse().unwrap())).await.unwrap();
        let providers = app.modules().contributions::<dyn VoiceProvider>();
        assert_eq!(providers.len(), 1);
        let outbound = OutboundCall {
            call_id: uuid::Uuid::now_v7(),
            from: PhoneNumber::parse("02838221234", "84").unwrap(),
            to: PhoneNumber::parse("+84901234567", "84").unwrap(),
            agent_identity: "agent".to_owned(),
            record: false,
            recording_notice: None,
        };
        let placed = providers[0].place_call(sandbox.id(), &outbound).await.unwrap();
        assert!(placed.provider_call_id.starts_with("SANDBOX-"), "{placed:?}");
        assert!(fakes.voice.calls().is_empty(), "không gọi nhà cung cấp thật");
        let (_, held) =
            call(&app, Method::GET, "/kernel/sandbox/messages", &token, &id, None).await;
        let text = held.to_string();
        assert!(text.contains("voice.call") && !text.contains("901234567"), "{text}");
        providers[0].place_call(shop.id(), &outbound).await.unwrap();
        assert_eq!(fakes.voice.calls().len(), 1, "khách thật gọi nhà cung cấp thật");
    })
    .await
    .unwrap();
}
