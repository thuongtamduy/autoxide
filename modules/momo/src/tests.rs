//! Test bộ nối MoMo (cả SQLite và Postgres) với máy chủ giả dựng theo tài liệu của MoMo, gọi thẳng
//! bộ nối: mã lỗi của MoMo ánh xạ sang `code` ổn định, truy vấn trạng thái, thử kết nối. Luồng qua
//! `ledger` (tạo giao dịch, IPN, hoàn tiền, hai khách) test ở `crates/app/src/e2e/momo.rs`.

use autoxide_kernel::connections::ConnectionStatus;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::payment::{GatewayPaymentStatus, PaymentGateway, PaymentIntent, RefundIntent};
use autoxide_kernel::provider::ProviderAccounts;
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{FakeProviderServer, ProviderRequest, TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::account::MomoConfig;
use crate::gateway::MomoGateway;

const ALL: [&str; 1] = ["kernel.settings.manage"];
const PUBLIC: &str = "https://api.shop.example";
const CREATE: &str = "/v2/gateway/api/create";
const QUERY: &str = "/v2/gateway/api/query";
const REFUND: &str = "/v2/gateway/api/refund";

/// Bộ khoá của một khách: (Partner Code, Access Key, Secret Key).
type Keys = (&'static str, &'static str, &'static str);
const ONE: Keys = ("MOMOONE", "access-of-one", "secret-of-one");

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
        .module(crate::module())
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
}

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

fn field(body: &Value, name: &str) -> String {
    match &body[name] {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Bộ nối dựng thẳng (không qua `ledger`), để gọi các hàm `ledger` không tự gọi.
fn gateway(setup: &Setup) -> MomoGateway {
    MomoGateway::new(
        (*setup.app.api::<ProviderAccounts>().unwrap()).clone(),
        MomoConfig {
            api_url: setup.momo.url().to_owned(),
            sandbox_api_url: setup.sandbox.url().to_owned(),
        },
        ConnectionStatus,
    )
    .unwrap()
}

fn only(requests: Vec<ProviderRequest>) -> ProviderRequest {
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests.into_iter().next().unwrap()
}
#[tokio::test]
async fn ma_loi_cua_momo_thanh_code_on_dinh() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        let tenant = app.tenant("one").await.unwrap();
        let gateway = gateway(&setup);
        let expires = Timestamp::now();
        let intent = |amount: Money| {
            PaymentIntent::new(Uuid::now_v7(), amount, "wallet", "Order", expires)
                .return_url(Some("https://shop.example/done".to_owned()))
        };
        let create = async |amount: Money| {
            gateway.create_payment(tenant.id(), &intent(amount)).await.unwrap_err()
        };

        // Kiểm trước khi gọi MoMo: tiền tệ và khoảng số tiền theo tài liệu.
        for (amount, code) in [
            (Money::new(1_000, Currency::USD), "momo.currency_not_supported"),
            (vnd(999), "momo.amount_out_of_range"),
            (vnd(50_000_001), "momo.amount_out_of_range"),
        ] {
            let refused = create(amount).await;
            assert_eq!((refused.code(), refused.kind().status()), (code, 422));
        }
        assert!(momo.take_requests().is_empty());

        // Mã kết quả của MoMo → `code` của Autoxide. Máy chủ giả trả các câu theo thứ tự đã xếp.
        let cases = [
            (13, "momo.account_rejected", 422),
            (11, "momo.account_rejected", 422),
            (22, "payment.provider_rejected", 422),
            (10, "payment.provider_unavailable", 503),
            (7000, "payment.provider_unavailable", 503),
            (41, "momo.order_already_created", 409),
            (40, "momo.order_already_created", 409),
        ];
        for (result_code, _, _) in cases {
            momo.reply("POST", CREATE, 200, json!({"resultCode": result_code, "message": "x"}));
        }
        for (result_code, code, http) in cases {
            let refused = create(vnd(20_000)).await;
            assert_eq!((refused.code(), refused.kind().status()), (code, http), "{result_code}");
            // Lỗi không bao giờ mang khoá của khách.
            assert!(!refused.message().contains(ONE.1) && !refused.message().contains(ONE.2));
        }
        // MoMo trả lỗi máy chủ không có JSON; MoMo không trả lời.
        let broken = FakeProviderServer::start().await.unwrap();
        broken.reply_raw("POST", CREATE, 502, "text/html", "Bad Gateway");
        let offline = MomoGateway::new(
            (*app.api::<ProviderAccounts>().unwrap()).clone(),
            MomoConfig {
                api_url: broken.url().to_owned(),
                // Cổng 9 (discard) không có ai nghe: kết nối bị từ chối ngay.
                sandbox_api_url: "http://127.0.0.1:9".to_owned(),
            },
            ConnectionStatus,
        )
        .unwrap();
        let refused = offline.create_payment(tenant.id(), &intent(vnd(20_000))).await.unwrap_err();
        assert_eq!(refused.code(), "payment.provider_unavailable");
        set(app, "one", "environment", "sandbox").await;
        let refused = offline.create_payment(tenant.id(), &intent(vnd(20_000))).await.unwrap_err();
        assert_eq!(refused.code(), "payment.provider_unavailable");
        set(app, "one", "environment", "production").await;

        // Hoàn tiền: lệnh đã tới MoMo ở lần trước (41) không được gửi thành lệnh mới; 1080 thử lại.
        momo.reply("POST", QUERY, 200, json!({"resultCode": 0, "amount": 20_000, "transId": 77}));
        let refund = RefundIntent::new(Uuid::now_v7(), "order-1", vnd(5_000), "Return");
        let cases = [(41, "momo.refund_already_sent"), (1080, "payment.provider_unavailable")];
        for (result_code, _) in cases {
            momo.reply("POST", REFUND, 200, json!({"resultCode": result_code}));
        }
        for (_, code) in cases {
            assert_eq!(gateway.refund(tenant.id(), &refund).await.unwrap_err().code(), code);
        }
        // Giao dịch gốc chưa thu được thì không gửi lệnh hoàn. (Câu "đã thu" còn ở đầu hàng của
        // máy chủ giả: một lần gọi lấy nó ra.)
        momo.reply("POST", QUERY, 200, json!({"resultCode": 1000, "message": "Waiting."}));
        assert!(gateway.refund(tenant.id(), &refund).await.is_err());
        momo.take_requests();
        let refused = gateway.refund(tenant.id(), &refund).await.unwrap_err();
        assert_eq!(refused.code(), "payment.provider_rejected");
        assert_eq!(only(momo.take_requests()).path, QUERY);

        // MoMo không có sao kê.
        let today = autoxide_kernel::time::Date::today_utc();
        let refused = gateway.statement(tenant.id(), today, today, None).await.unwrap_err();
        assert_eq!(refused.code(), "payment.not_supported");
        assert!(!gateway.capabilities().statements);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn truy_van_trang_thai_theo_ma_ket_qua() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        configure(app, "one", ONE).await;
        let tenant = app.tenant("one").await.unwrap();
        let gateway = gateway(&setup);
        let codes = [0, 1000, 7002, 9000, 1005, 1006, 42];
        for result_code in codes {
            let answer = json!({"resultCode": result_code, "amount": 150_000,
                                "transId": 4_088_878_653_i64, "message": "m"});
            momo.reply("POST", QUERY, 200, answer);
        }
        momo.reply("POST", QUERY, 200, json!({"resultCode": 13}));
        let mut seen = Vec::new();
        for _ in codes {
            seen.push(gateway.payment_status(tenant.id(), "order-1").await.unwrap());
        }
        assert_eq!(seen[0], GatewayPaymentStatus::Succeeded { amount: vnd(150_000) });
        assert_eq!(seen[1..4], [const { GatewayPaymentStatus::Pending }; 3]);
        assert_eq!(seen[4], GatewayPaymentStatus::Expired);
        for failed in &seen[5..] {
            assert!(matches!(failed, GatewayPaymentStatus::Failed { .. }), "{failed:?}");
        }
        // Tài khoản bị từ chối là lỗi, không phải "giao dịch thất bại".
        let refused = gateway.payment_status(tenant.id(), "order-1").await.unwrap_err();
        assert_eq!(refused.code(), "momo.account_rejected");
        let request = momo.take_requests().pop().unwrap().json();
        assert_eq!(
            (&request["orderId"], &request["partnerCode"]),
            (&json!("order-1"), &json!(ONE.0))
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thu_ket_noi_tu_trang_cai_dat() {
    TestDatabase::run(async |test| {
        let setup = setup(test).await;
        let (app, momo) = (&setup.app, &setup.momo);
        let path = "/api/v1/kernel/settings/momo.connection.account.secret_key/test";
        let test_connection = async |token: &str| {
            let answer = app.post(path).token(token).send().await.unwrap();
            assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
            answer.json()["data"].clone()
        };
        // Chưa nhập tài khoản: kết quả nói rõ còn thiếu gì, không gọi MoMo.
        let result = test_connection("one").await;
        assert_eq!(result["ok"], false, "{result}");
        assert!(result["error"].as_str().unwrap().contains("partner_code"), "{result}");
        assert!(momo.take_requests().is_empty());

        configure(app, "one", ONE).await;
        // MoMo nhận chữ ký và trả "không tìm thấy giao dịch" (42): tài khoản dùng được.
        momo.reply("POST", QUERY, 200, json!({"resultCode": 42, "message": "Not found."}));
        momo.reply("POST", QUERY, 200, json!({"resultCode": 13, "message": "Auth failed."}));
        let result = test_connection("one").await;
        assert_eq!(
            (&result["ok"], &result["connection"]),
            (&json!(true), &json!("momo.connection.account"))
        );
        let request = only(momo.take_requests()).json();
        assert_eq!(request["partnerCode"], ONE.0);
        assert!(field(&request, "orderId").starts_with("probe-"));

        // Sai khoá (13): báo lỗi cho người quản trị, không lộ khoá.
        let result = test_connection("one").await;
        assert_eq!(result["ok"], false);
        let error = result["error"].as_str().unwrap();
        assert!(error.contains("13") && !error.contains(ONE.2), "{error}");
    })
    .await
    .unwrap();
}
