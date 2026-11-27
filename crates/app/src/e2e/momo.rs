//! Bộ nối MoMo trên chương trình thật (cả SQLite và Postgres), với máy chủ giả dựng theo tài liệu của
//! MoMo và `ledger` thật: request và chữ ký đúng tài liệu, hai khách hai tài khoản không lẫn nhau,
//! khách chưa cấu hình báo lỗi rõ, IPN sai chữ ký bị từ chối, IPN gửi lại không xử lý trùng, số
//! tiền lệch vào hàng chờ đối soát, hoàn tiền. Mã lỗi và truy vấn trạng thái test ở crate `momo`.

use std::sync::Arc;

use autoxide_kernel::event::Actor;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::provider::{hex, hmac_sha256};
use autoxide_kernel::tenancy::{TenantContext, root_of};
use autoxide_ledger::api::{AccountKey, Counterparty, Ledger, NewPayment, NewRefund, Payment};
use autoxide_testkit::{FakeProviderServer, ProviderRequest, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use uuid::Uuid;

const ALL: [&str; 5] = [
    "kernel.settings.manage",
    "ledger.read",
    "ledger.payments_create",
    "ledger.refunds_create",
    "ledger.reconcile",
];
const PUBLIC: &str = "https://api.shop.example";
const CREATE: &str = "/v2/gateway/api/create";
const QUERY: &str = "/v2/gateway/api/query";
const REFUND: &str = "/v2/gateway/api/refund";

/// Bộ khoá của một khách: (Partner Code, Access Key, Secret Key).
type Keys = (&'static str, &'static str, &'static str);
const ONE: Keys = ("MOMOONE", "access-of-one", "secret-of-one");
const TWO: Keys = ("MOMOTWO", "access-of-two", "secret-of-two");

struct Setup {
    app: TestApp,
    /// Máy chủ giả của môi trường production.
    momo: FakeProviderServer,
    /// Máy chủ giả của môi trường sandbox.
    sandbox: FakeProviderServer,
}

async fn setup(test: &TestDatabase) -> Setup {
    let momo = FakeProviderServer::start().await.unwrap();
    let sandbox = FakeProviderServer::start().await.unwrap();
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_MOMO_API_URL", momo.url())
        .setting("AUTOXIDE_MOMO_SANDBOX_API_URL", sandbox.url())
        .setting("AUTOXIDE_HTTP_PUBLIC_URL", PUBLIC)
        .tenant("one")
        .tenant("two")
        .user_in("one", "one", 1, &ALL)
        .user_in("two", "two", 2, &ALL)
        // Tên miền đã xác minh của khách hai: trang quay về trên đó được nhận (P-5).
        .domain("two", "two.example")
        // Bộ nối đăng ký TRƯỚC `ledger`, như trong binary.
        .module(autoxide_momo::module())
        .module(autoxide_ledger::module())
        .build()
        .await
        .unwrap();
    Setup { app, momo, sandbox }
}

async fn set(app: &TestApp, token: &str, field: &str, value: &str) -> (StatusCode, Value) {
    let path = format!("/api/v1/kernel/settings/momo.connection.account.{field}");
    let answer = app.put(&path).token(token).json(json!({"value": value})).send().await.unwrap();
    (answer.status(), answer.json())
}

/// Khách có token `token` nhập tài khoản MoMo của mình qua API cấu hình.
async fn configure(app: &TestApp, token: &str, keys: Keys) {
    for (field, value) in [("partner_code", keys.0), ("access_key", keys.1), ("secret_key", keys.2)]
    {
        let (status, body) = set(app, token, field, value).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    allow_return(app, token).await;
}

/// Trang quay về sau khi trả phải thuộc cửa hàng (P-5, D-20261125-return-url-allowlist): khách có token
/// `token` khai `https://shop.example`.
async fn allow_return(app: &TestApp, token: &str) {
    let path = "/api/v1/kernel/settings/ledger.payments.return_origins";
    let answer = app
        .put(path)
        .token(token)
        .json(json!({"value": "https://shop.example"}))
        .send()
        .await
        .unwrap();
    assert_eq!(answer.status(), StatusCode::OK, "{}", answer.json());
}

fn ledger(app: &TestApp) -> Arc<dyn Ledger> {
    app.api::<dyn Ledger>().unwrap()
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

fn new_payment(tenant: &TenantContext, amount: Money, reference: &str) -> NewPayment {
    NewPayment::new(root_of(tenant.id()), "wallet", amount, "Order 1001", "demo", reference)
        .provider("momo")
        .return_url("https://shop.example/done")
}

async fn pay(
    app: &TestApp,
    slug: &str,
    amount: i64,
    reference: &str,
) -> Result<Payment, autoxide_kernel::error::Error> {
    let tenant = app.tenant(slug).await.unwrap();
    let payment = new_payment(&tenant, vnd(amount), reference);
    Ok(ledger(app).create_payment(&tenant, Actor::system(), payment).await?.into_inner())
}

fn created() -> Value {
    json!({
        "partnerCode": "MOMOONE", "resultCode": 0, "message": "Thành công.",
        "payUrl": "https://test-payment.momo.vn/pay/abc",
        "deeplink": "momo://app?action=pay", "qrCodeUrl": "00020101021226110007vn.momo",
        "responseTime": 1_795_000_000_000_i64,
    })
}

/// Chữ ký theo tài liệu MoMo, tính độc lập với code của bộ nối: `raw` là chuỗi `key=value&…`.
fn sign(secret: &str, raw: &str) -> String {
    hex(&hmac_sha256(secret.as_bytes(), raw.as_bytes()))
}

fn field(body: &Value, name: &str) -> String {
    match &body[name] {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// URL nhận IPN của khách có token `token`, như khách thấy ở trang cài đặt.
async fn webhook_path(app: &TestApp, token: &str) -> String {
    let listed = app.get("/api/v1/kernel/connections/webhooks").token(token).send().await.unwrap();
    let listed = listed.json();
    let row = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["connection"] == "momo.connection.account")
        .unwrap_or_else(|| panic!("{listed}"))
        .clone();
    row["path"].as_str().unwrap().to_owned()
}

/// Thân IPN theo tài liệu MoMo cho thanh toán `order_id`, ký bằng bộ khoá `keys`.
fn ipn(keys: Keys, order_id: &str, amount: i64, trans_id: i64, result_code: i64) -> Value {
    let (message, order_info, order_type, pay_type) =
        ("Thành công.", "Order 1001", "momo_wallet", "qr");
    let (request_id, response_time, extra_data) = (order_id, 1_795_000_100_000_i64, "");
    let raw = format!(
        "accessKey={}&amount={amount}&extraData={extra_data}&message={message}&orderId={order_id}\
         &orderInfo={order_info}&orderType={order_type}&partnerCode={}&payType={pay_type}\
         &requestId={request_id}&responseTime={response_time}&resultCode={result_code}\
         &transId={trans_id}",
        keys.1, keys.0
    );
    json!({
        "partnerCode": keys.0, "orderId": order_id, "requestId": request_id, "amount": amount,
        "orderInfo": order_info, "orderType": order_type, "transId": trans_id,
        "resultCode": result_code, "message": message, "payType": pay_type,
        "responseTime": response_time, "extraData": extra_data, "signature": sign(keys.2, &raw),
    })
}

/// Gửi IPN KHÔNG kèm đăng nhập hay header chọn khách — như MoMo gửi.
async fn push(app: &TestApp, path: &str, body: &Value) -> (StatusCode, Value) {
    let answer = app
        .request(Method::POST, path)
        .header("content-type", "application/json")
        .bytes(body.to_string().into_bytes())
        .send()
        .await
        .unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Chạy hết job đang chờ (xử lý thông báo).
async fn drain(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    for _ in 0..5 {
        if worker.run_once().await.unwrap() == 0 {
            break;
        }
    }
}

async fn status(app: &TestApp, slug: &str, id: Uuid) -> String {
    let tenant = app.tenant(slug).await.unwrap();
    ledger(app).payment(&tenant, id).await.unwrap().unwrap().status
}

/// Số dư tiền đang nằm ở MoMo theo sổ của khách `slug`.
async fn clearing(app: &TestApp, slug: &str) -> i64 {
    let tenant = app.tenant(slug).await.unwrap();
    let key = AccountKey::new(root_of(tenant.id()), "clearing.gateway", Currency::VND)
        .with_counterparty(Counterparty::new("partner", "momo"));
    ledger(app).balance(&tenant, key, None).await.unwrap().minor()
}

fn only(requests: Vec<ProviderRequest>) -> ProviderRequest {
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests.into_iter().next().unwrap()
}
#[tokio::test]
async fn tao_giao_dich_gui_dung_request_va_chu_ky_cua_tung_khach() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        configure(app, "two", TWO).await;
        momo.reply("POST", CREATE, 200, created());

        let mut ipn_urls = Vec::new();
        for (slug, keys) in [("one", ONE), ("two", TWO)] {
            let payment = pay(app, slug, 150_000, "P-1").await.unwrap();
            let request = only(momo.take_requests());
            assert_eq!((request.method.as_str(), request.path.as_str()), ("POST", CREATE));
            assert!(request.header("content-type").unwrap().starts_with("application/json"));
            let body = request.json();
            let id = payment.id.to_string();
            assert_eq!(body["partnerCode"], keys.0);
            // `orderId` = `requestId` = ID thanh toán trong sổ; số tiền là số nguyên VND.
            assert_eq!((&body["orderId"], &body["requestId"]), (&json!(id), &json!(id)));
            assert_eq!(body["amount"], json!(150_000));
            assert_eq!(body["requestType"], "captureWallet");
            assert_eq!(body["orderInfo"], "Order 1001");
            assert_eq!(body["redirectUrl"], "https://shop.example/done");
            assert_eq!(body["extraData"], "");
            // `ipnUrl` là URL webhook riêng của khách, trên địa chỉ công khai của máy chủ.
            let ipn_url = field(&body, "ipnUrl");
            assert_eq!(ipn_url, format!("{PUBLIC}{}", webhook_path(app, slug).await));
            // Chuỗi ký đúng thứ tự trong tài liệu, ký bằng Secret Key của ĐÚNG khách.
            let raw = format!(
                "accessKey={}&amount=150000&extraData=&ipnUrl={ipn_url}&orderId={id}\
                 &orderInfo=Order 1001&partnerCode={}&redirectUrl=https://shop.example/done\
                 &requestId={id}&requestType=captureWallet",
                keys.1, keys.0
            );
            assert_eq!(body["signature"], sign(keys.2, &raw), "{slug}");
            // Khoá không nằm trong thân request.
            let sent = String::from_utf8_lossy(&request.body).into_owned();
            assert!(!sent.contains(keys.1) && !sent.contains(keys.2), "{sent}");

            assert_eq!(payment.provider_ref.as_deref(), Some(id.as_str()));
            assert_eq!(
                payment.checkout_url.as_deref(),
                Some("https://test-payment.momo.vn/pay/abc")
            );
            assert_eq!(payment.qr_payload.as_deref(), Some("00020101021226110007vn.momo"));
            ipn_urls.push(ipn_url);
        }
        assert_ne!(ipn_urls[0], ipn_urls[1]);

        // Gọi lại cùng `reference`: thanh toán đã có mã ở MoMo, không gọi MoMo lần nữa.
        let again = pay(app, "one", 150_000, "P-1").await.unwrap();
        assert!(again.checkout_url.is_some());
        assert!(momo.take_requests().is_empty());
        // Secret không bao giờ được trả lại qua API.
        let listed =
            app.get("/api/v1/kernel/settings?group=connection").token("one").send().await.unwrap();
        let text = listed.text();
        assert!(text.contains("momo.connection.account.secret_key"), "{text}");
        assert!(!text.contains(ONE.1) && !text.contains(ONE.2), "{text}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_chua_cau_hinh_bao_loi_ro_va_khong_dung_tai_khoan_khac() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        momo.reply("POST", CREATE, 200, created());

        // Khách hai chưa nhập tài khoản MoMo: lỗi nêu trường còn thiếu, không có request nào ra MoMo.
        allow_return(app, "two").await;
        let refused = pay(app, "two", 150_000, "P-1").await.unwrap_err();
        assert_eq!((refused.code(), refused.kind().status()), ("provider.not_configured", 422));
        assert!(refused.message().contains("momo.connection.account.partner_code"));
        assert!(momo.take_requests().is_empty());

        // Giá trị sai dạng bị từ chối ngay lúc đặt.
        let (status, body) = set(app, "two", "environment", "staging").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let (status, body) = set(app, "two", "return_url", "not a url").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");

        // Thanh toán không kèm trang quay về và khách chưa đặt trang mặc định.
        let one = app.tenant("one").await.unwrap();
        let bare =
            NewPayment::new(root_of(one.id()), "wallet", vnd(50_000), "Order", "demo", "B-1")
                .provider("momo");
        let refused = ledger(app).create_payment(&one, Actor::system(), bare.clone()).await;
        assert_eq!(refused.unwrap_err().code(), "momo.return_url_required");
        assert!(momo.take_requests().is_empty());
        // Đặt trang mặc định của khách rồi gọi lại cùng `reference`: dùng trang đó.
        let (status, _) = set(app, "one", "return_url", "https://shop.example/thanks").await;
        assert_eq!(status, StatusCode::OK);
        ledger(app).create_payment(&one, Actor::system(), bare).await.unwrap();
        assert_eq!(only(momo.take_requests()).json()["redirectUrl"], "https://shop.example/thanks");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_dung_moi_truong_sandbox_goi_may_chu_thu() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let app = &setup.app;
        configure(app, "one", ONE).await;
        configure(app, "two", TWO).await;
        let (status, body) = set(app, "two", "environment", "sandbox").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        setup.momo.reply("POST", CREATE, 200, created());
        setup.sandbox.reply("POST", CREATE, 200, created());

        pay(app, "one", 20_000, "P-1").await.unwrap();
        assert_eq!(setup.momo.take_requests().len(), 1);
        assert!(setup.sandbox.take_requests().is_empty());
        pay(app, "two", 20_000, "P-1").await.unwrap();
        assert!(setup.momo.take_requests().is_empty());
        assert_eq!(only(setup.sandbox.take_requests()).json()["partnerCode"], TWO.0);
    })
    .await
    .unwrap();
}

/// P-5 (pentest 2026-11): trang quay về sau khi trả chỉ nhận địa chỉ của chính cửa hàng — tên miền đã xác
/// minh (https) hay origin khách khai; địa chỉ lạ bị từ chối trước khi gọi MoMo (không thành open redirect).
#[tokio::test]
async fn trang_quay_ve_phai_thuoc_cua_hang() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        configure(app, "two", TWO).await;
        momo.reply("POST", CREATE, 200, created());
        let attempt = async |slug: &str, url: &str, reference: &str| {
            let tenant = app.tenant(slug).await.unwrap();
            let payment = new_payment(&tenant, vnd(20_000), reference).return_url(url);
            ledger(app).create_payment(&tenant, Actor::system(), payment).await
        };

        for url in [
            "https://evil.example/done",
            "https://shop.example.evil.example/done",
            "https://shop.example@evil.example/done",
            "http://shop.example:8080/done",
            "https://two.example/done",
            // Lớp lách (kiểm độc lập #508): IDN giống mặt chữ, host mã hoá `%`, IPv6, khoảng trắng / ký tự điều
            // khiển, thiếu scheme, scheme khác.
            "https://shöp.example/done",
            "https://evil%2eexample/done",
            "https://[::1]/done",
            " https://shop.example/done",
            "https://shop.example/do\nne",
            "//shop.example/done",
            "ftp://shop.example/done",
        ] {
            let refused = attempt("one", url, "P-1").await.unwrap_err();
            assert_eq!(refused.code(), "validation_failed", "{url}");
            // `user@host` không phải URL quay về hợp lệ (`format`); địa chỉ lạ khác: không thuộc cửa hàng.
            let field = &refused.fields()[0];
            assert_eq!(field.field, "return_url", "{url}");
            assert!(
                ["format", "ledger.return_url_not_allowed"].contains(&field.code.as_str()),
                "{url}"
            );
        }
        assert!(momo.take_requests().is_empty(), "không gọi MoMo với địa chỉ lạ");
        attempt("one", "https://SHOP.example:443/thanks?order=1", "P-2").await.unwrap();
        // Cùng máy chủ viết khác (dấu chấm cuối, `%2e`): nhận, và đối tác nhận chuỗi đã chuẩn hoá.
        attempt("one", "https://shop.example./thanks", "P-6").await.unwrap();
        attempt("one", "https://shop%2eexample/thanks", "P-7").await.unwrap();
        let sent: Vec<String> = momo
            .take_requests()
            .iter()
            .filter_map(|request| request.json()["redirectUrl"].as_str().map(str::to_owned))
            .collect();
        assert!(sent.contains(&"https://shop.example/thanks?order=1".to_owned()), "{sent:?}");
        assert!(sent.contains(&"https://shop.example/thanks".to_owned()), "{sent:?}");

        // Khách hai không khai origin nào: tên miền đã xác minh của mình (https) vẫn được nhận.
        let path = "/api/v1/kernel/settings/ledger.payments.return_origins";
        let cleared = app.delete(path).token("two").send().await.unwrap();
        assert!(cleared.status().is_success(), "{}", cleared.json());
        attempt("two", "https://two.example/thanks", "P-3").await.unwrap();
        let refused = attempt("two", "http://two.example/thanks", "P-4").await.unwrap_err();
        assert_eq!(refused.fields()[0].code, "ledger.return_url_not_allowed");
        assert!(attempt("two", "https://shop.example/thanks", "P-5").await.is_err());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ipn_kiem_chu_ky_cua_dung_khach_va_khong_xu_ly_trung() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        configure(app, "two", TWO).await;
        momo.reply("POST", CREATE, 200, created());
        let payment = pay(app, "one", 150_000, "P-1").await.unwrap();
        let id = payment.id.to_string();
        let (one, two) = (webhook_path(app, "one").await, webhook_path(app, "two").await);
        assert!(one.starts_with("/api/v1/ledger/webhooks/momo/"), "{one}");

        // Ký bằng khoá của khách hai, gửi tới URL của khách một: sai chữ ký.
        let (status, body) = push(app, &one, &ipn(TWO, &id, 150_000, 4_088_878_653, 0)).await;
        assert_eq!(
            (status, &body["code"]),
            (StatusCode::UNAUTHORIZED, &json!("payment.signature_invalid"))
        );
        // Ký bằng khoá của khách một, gửi tới URL của khách hai: cũng sai.
        let (status, _) = push(app, &two, &ipn(ONE, &id, 150_000, 4_088_878_653, 0)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        // Sửa số tiền sau khi ký; bỏ chữ ký; thân không phải JSON.
        let mut tampered = ipn(ONE, &id, 150_000, 4_088_878_653, 0);
        tampered["amount"] = json!(1_000);
        let mut unsigned = ipn(ONE, &id, 150_000, 4_088_878_653, 0);
        unsigned.as_object_mut().unwrap().remove("signature");
        for body in [tampered, unsigned, json!("hello")] {
            let (status, _) = push(app, &one, &body).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        }
        // Kết quả chưa cuối (đang chờ người mua): không ghi gì.
        let (status, body) = push(app, &one, &ipn(ONE, &id, 150_000, 4_088_878_653, 1000)).await;
        assert_eq!(
            (status, &body["code"]),
            (StatusCode::UNPROCESSABLE_ENTITY, &json!("momo.notification_not_final"))
        );
        drain(app).await;
        assert_eq!(status_of(app, &payment).await, "pending");

        // IPN đúng: MoMo chờ HTTP 204, thân rỗng. Gửi lại (MoMo gửi lại khi chưa nhận 204) không
        // ghi sổ lần hai.
        let paid = ipn(ONE, &id, 150_000, 4_088_878_653, 0);
        for _ in 0..3 {
            let (status, body) = push(app, &one, &paid).await;
            assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
        }
        drain(app).await;
        assert_eq!(status_of(app, &payment).await, "succeeded");
        assert_eq!(clearing(app, "one").await, 150_000);
        assert_eq!(clearing(app, "two").await, 0);
        // Thông báo "thất bại" tới muộn cho cùng giao dịch không làm lùi trạng thái.
        let (status, _) = push(app, &one, &ipn(ONE, &id, 150_000, 4_088_878_653, 1006)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        drain(app).await;
        assert_eq!(status_of(app, &payment).await, "succeeded");
        assert_eq!(clearing(app, "one").await, 150_000);
    })
    .await
    .unwrap();
}

async fn status_of(app: &TestApp, payment: &Payment) -> String {
    status(app, "one", payment.id).await
}

#[tokio::test]
async fn ipn_bao_so_tien_khac_thi_cho_nguoi_doi_soat() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        momo.reply("POST", CREATE, 200, created());
        let payment = pay(app, "one", 150_000, "P-1").await.unwrap();
        let one = webhook_path(app, "one").await;
        // Chữ ký đúng nhưng MoMo báo thu 100.000 thay vì 150.000: không tự ghi sổ.
        let short = ipn(ONE, &payment.id.to_string(), 100_000, 4_088_878_654, 0);
        assert_eq!(push(app, &one, &short).await.0, StatusCode::NO_CONTENT);
        drain(app).await;
        assert_eq!(status_of(app, &payment).await, "needs_review");
        assert_eq!(clearing(app, "one").await, 0);

        // Thông báo "đúng số" gửi sau cho cùng giao dịch (cùng `transId`): bỏ qua, không gỡ được lệch
        // (UT-6 L5) — chỉ người đối soát gỡ.
        let corrected = ipn(ONE, &payment.id.to_string(), 150_000, 4_088_878_654, 0);
        assert_eq!(push(app, &one, &corrected).await.0, StatusCode::NO_CONTENT);
        drain(app).await;
        assert_eq!(status_of(app, &payment).await, "needs_review");

        // Người đối soát: mục `amount_mismatch` đang mở → nhận số thực thu → đã thu đúng số đó.
        let listed = app
            .get("/api/v1/ledger/exceptions?status=open")
            .token("one")
            .send()
            .await
            .unwrap()
            .json();
        let item = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["kind"] == "amount_mismatch")
            .unwrap_or_else(|| panic!("{listed}"))
            .clone();
        assert_eq!(item["subject"], format!("ledger.payment:{}", payment.id));
        let resolved = app
            .post(&format!("/api/v1/ledger/exceptions/{}/resolve", item["id"].as_str().unwrap()))
            .token("one")
            .json(json!({"action": "accept_received", "payment_id": payment.id,
                         "note": "MoMo confirmed 100,000 by phone"}))
            .send()
            .await
            .unwrap();
        assert_eq!(resolved.status(), StatusCode::OK, "{}", resolved.json());
        assert_eq!(resolved.json()["data"]["status"], "resolved");
        drain(app).await;
        assert_eq!(status_of(app, &payment).await, "succeeded");
        assert_eq!(clearing(app, "one").await, 100_000);

        // Thanh toán thất bại ở MoMo (người mua từ chối, mã 1006).
        let other = pay(app, "one", 80_000, "P-2").await.unwrap();
        let denied = ipn(ONE, &other.id.to_string(), 80_000, 4_088_878_655, 1006);
        assert_eq!(push(app, &one, &denied).await.0, StatusCode::NO_CONTENT);
        drain(app).await;
        assert_eq!(status(app, "one", other.id).await, "failed");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hoan_tien_hoi_ma_giao_dich_roi_gui_lenh_hoan() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        momo.reply("POST", CREATE, 200, created());
        let payment = pay(app, "one", 150_000, "P-1").await.unwrap();
        let id = payment.id.to_string();
        let one = webhook_path(app, "one").await;
        push(app, &one, &ipn(ONE, &id, 150_000, 4_088_878_653, 0)).await;
        drain(app).await;
        momo.take_requests();

        let tenant = app.tenant("one").await.unwrap();
        let paid = json!({"partnerCode": ONE.0, "orderId": id, "resultCode": 0, "amount": 150_000,
                          "transId": 4_088_878_653_i64, "payType": "qr", "message": "Thành công."});
        momo.reply("POST", QUERY, 200, paid);
        momo.reply(
            "POST",
            REFUND,
            200,
            json!({"partnerCode": ONE.0, "resultCode": 0,
            "transId": 4_088_900_000_i64, "amount": 50_000, "message": "Thành công."}),
        );
        momo.reply("POST", REFUND, 200, json!({"resultCode": 1081, "message": "Rejected."}));
        let refund = NewRefund::new(payment.id, vnd(50_000), "Customer returned one item", "R-1");
        let refund =
            ledger(app).refund(&tenant, Actor::system(), refund).await.unwrap().into_inner();
        assert_eq!(refund.status, "succeeded");
        let refund_id = refund.id.to_string();
        assert_eq!(refund.provider_ref.as_deref(), Some(refund_id.as_str()));

        let requests = momo.take_requests();
        assert_eq!(requests.len(), 2, "{requests:?}");
        // 1. Hỏi giao dịch gốc theo `orderId` để lấy `transId`.
        let query = requests[0].json();
        assert_eq!((requests[0].path.as_str(), &query["orderId"]), (QUERY, &json!(id)));
        let request_id = field(&query, "requestId");
        let raw = format!(
            "accessKey={}&orderId={id}&partnerCode={}&requestId={request_id}",
            ONE.1, ONE.0
        );
        assert_eq!(query["signature"], sign(ONE.2, &raw));
        // 2. Lệnh hoàn: `orderId` mới (ID lượt hoàn), `transId` của giao dịch gốc, số nguyên VND.
        let sent = requests[1].json();
        assert_eq!(requests[1].path, REFUND);
        assert_eq!((&sent["orderId"], &sent["requestId"]), (&json!(refund_id), &json!(refund_id)));
        assert_eq!(
            (&sent["amount"], &sent["transId"]),
            (&json!(50_000), &json!(4_088_878_653_i64))
        );
        let raw = format!(
            "accessKey={}&amount=50000&description=Customer returned one item&orderId={refund_id}\
             &partnerCode={}&requestId={refund_id}&transId=4088878653",
            ONE.1, ONE.0
        );
        assert_eq!(sent["signature"], sign(ONE.2, &raw));

        // MoMo từ chối (1081: đã hoàn / vượt số được hoàn) → `code` ổn định.
        let refused = NewRefund::new(payment.id, vnd(10_000), "Again", "R-2");
        let refused = ledger(app).refund(&tenant, Actor::system(), refused).await.unwrap_err();
        assert_eq!(refused.code(), "payment.provider_rejected");
        assert!(refused.message().contains("1081"), "{}", refused.message());
    })
    .await
    .unwrap();
}
