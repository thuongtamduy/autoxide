//! Test của bộ nối eSMS với máy chủ giả dựng theo tài liệu (`FakeProviderServer`). Chạy trên cả hai
//! database: tài khoản của khách nằm trong kho cấu hình theo khách.

use std::sync::Arc;

use autoxide_kernel::error::ErrorKind;
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose, SmsSender};
use autoxide_kernel::tenancy::TenantId;
use autoxide_testkit::http::StatusCode;
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};
use serde_json::{Value, json};

const SEND: &str = "/MainService.svc/json/SendMultipleMessage_V4_post_json/";
const BALANCE: &str = "/MainService.svc/json/GetBalance_json";
const OTP_TEXT: &str = "123456 la ma xac minh dang ky Baotrixemay cua ban";

struct Fixture {
    app: TestApp,
    server: FakeProviderServer,
    sender: Arc<dyn SmsSender>,
}

/// App có hai khách `one`, `two`; mỗi khách một người quản trị cấu hình (token `one-admin`, `two-admin`).
async fn fixture(test: &TestDatabase) -> Fixture {
    let server = FakeProviderServer::start().await.unwrap();
    let app = TestApp::builder(test)
        .setting("AUTOXIDE_ESMS_API_URL", server.url())
        .tenant("one")
        .tenant("two")
        .user_in("one", "one-admin", 1, &["kernel.settings.manage"])
        .user_in("two", "two-admin", 2, &["kernel.settings.manage"])
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let sender = app.api::<dyn SmsSender>().unwrap();
    Fixture { app, server, sender }
}

impl Fixture {
    async fn set(&self, token: &str, name: &str, value: &str) -> Value {
        let answer = self
            .app
            .put(&format!("/api/v1/kernel/settings/{name}"))
            .token(token)
            .json(json!({"value": value}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status_code(), StatusCode::OK, "{}", answer.json());
        assert!(
            !answer.text().contains(value) || !name.ends_with("_key"),
            "secret is never answered"
        );
        answer.json()
    }

    /// Khách `slug` nhập tài khoản eSMS của mình qua REST, như ngoài đời.
    async fn account(&self, slug: &str, brandname: &str) {
        let token = format!("{slug}-admin");
        self.set(&token, "esms.connection.sms.api_key", &format!("api-key-of-{slug}")).await;
        self.set(&token, "esms.connection.sms.secret_key", &format!("secret-key-of-{slug}")).await;
        self.set(&token, "esms.connection.sms.brandname", brandname).await;
    }

    async fn tenant(&self, slug: &str) -> TenantId {
        self.app.tenant(slug).await.unwrap().id()
    }

    async fn otp(&self, slug: &str, reference: &str) -> SmsMessage {
        SmsMessage::new(
            self.tenant(slug).await,
            PhoneNumber::parse("0901888484", "84").unwrap(),
            OTP_TEXT,
            SmsPurpose::Otp,
            reference,
        )
    }

    fn accepted(&self, sms_id: &str) {
        self.server.reply(
            "POST",
            SEND,
            200,
            json!({"CodeResult": "100", "CountRegenerate": 0, "SMSID": sms_id}),
        );
    }
}

#[tokio::test]
async fn gui_dung_tai_lieu_va_hai_khach_khong_lan_tai_khoan() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.account("one", "Baotrixemay").await;
        fx.account("two", "ShopHai").await;

        // Máy chủ giả trả các câu đã xếp theo thứ tự và giữ lại câu cuối: xếp MỌI câu trước khi gửi.
        fx.accepted("sms-of-one");
        fx.accepted("sms-of-two");
        let receipt = fx.sender.send(fx.otp("one", "ref-1").await).await.unwrap();
        assert_eq!(receipt.provider_reference, "sms-of-one");
        assert_eq!(fx.sender.provider(), "esms");

        let message = SmsMessage::new(
            fx.tenant("two").await,
            PhoneNumber::parse("+84912345678", "84").unwrap(),
            "Đơn hàng của bạn đã được giao",
            SmsPurpose::Transactional,
            "ref-2",
        );
        let receipt = fx.sender.send(message).await.unwrap();
        assert_eq!(receipt.provider_reference, "sms-of-two");

        let seen = fx.server.take_requests();
        assert_eq!(seen.len(), 2);
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("POST", SEND));
        assert_eq!(seen[0].header("content-type"), Some("application/json"));
        // Đúng các trường của tài liệu; key và brandname của ĐÚNG khách.
        assert_eq!(
            seen[0].json(),
            json!({
                "ApiKey": "api-key-of-one",
                "SecretKey": "secret-key-of-one",
                "Brandname": "Baotrixemay",
                "Phone": "0901888484",
                "Content": OTP_TEXT,
                "SmsType": "2",
                "IsUnicode": "0",
                "RequestId": "ref-1",
            })
        );
        assert_eq!(
            seen[1].json(),
            json!({
                "ApiKey": "api-key-of-two",
                "SecretKey": "secret-key-of-two",
                "Brandname": "ShopHai",
                "Phone": "0912345678",
                "Content": "Đơn hàng của bạn đã được giao",
                "SmsType": "2",
                "IsUnicode": "1",
                "RequestId": "ref-2",
            })
        );
        // Key không nằm trong URL.
        assert!(seen.iter().all(|request| request.query.is_empty()));

        // Secret không bao giờ được trả lại; brandname (không bí mật) thì có.
        let listed = fx
            .app
            .get("/api/v1/kernel/settings?group=connection")
            .token("one-admin")
            .send()
            .await
            .unwrap();
        let text = listed.text();
        assert!(!text.contains("api-key-of-one") && !text.contains("secret-key-of-one"), "{text}");
        let names: Vec<(String, bool, bool)> = listed.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["name"].as_str().unwrap().starts_with("esms."))
            .map(|s| {
                (s["name"].as_str().unwrap().to_owned(), s["secret"] == true, s["is_set"] == true)
            })
            .collect();
        assert_eq!(
            names,
            [
                ("esms.connection.sms.api_key".to_owned(), true, true),
                ("esms.connection.sms.brandname".to_owned(), false, true),
                ("esms.connection.sms.environment".to_owned(), false, false),
                ("esms.connection.sms.secret_key".to_owned(), true, true),
            ]
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_chua_cau_hinh_bao_loi_ro_va_khong_dung_tai_khoan_khach_khac() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.account("one", "Baotrixemay").await;

        // `two` chưa nhập gì: không gọi eSMS, không mượn tài khoản của `one`.
        let error = fx.sender.send(fx.otp("two", "ref-1").await).await.unwrap_err();
        assert_eq!(error.code(), "provider.not_configured");
        assert_eq!(error.kind(), ErrorKind::Validation);
        assert!(error.message().contains("esms.connection.sms.api_key"), "{}", error.message());

        // Thiếu đúng brandname thì nêu đúng tên đó.
        fx.set("two-admin", "esms.connection.sms.api_key", "api-key-of-two").await;
        fx.set("two-admin", "esms.connection.sms.secret_key", "secret-key-of-two").await;
        let error = fx.sender.send(fx.otp("two", "ref-1").await).await.unwrap_err();
        assert_eq!(error.code(), "provider.not_configured");
        assert!(error.message().contains("esms.connection.sms.brandname"), "{}", error.message());
        assert!(fx.server.take_requests().is_empty());

        // Brandname rỗng và môi trường lạ bị từ chối lúc lưu.
        for (name, value) in
            [("esms.connection.sms.brandname", "   "), ("esms.connection.sms.environment", "test")]
        {
            let answer = fx
                .app
                .put(&format!("/api/v1/kernel/settings/{name}"))
                .token("two-admin")
                .json(json!({"value": value}))
                .send()
                .await
                .unwrap();
            assert_eq!(answer.status_code(), StatusCode::UNPROCESSABLE_ENTITY, "{}", answer.json());
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tin_quang_cao_va_so_ngoai_viet_nam_bi_tu_choi_truoc_khi_goi() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.account("one", "Baotrixemay").await;
        let tenant = fx.tenant("one").await;

        let marketing = SmsMessage::new(
            tenant,
            PhoneNumber::parse("0901888484", "84").unwrap(),
            "[QC] Giam gia 50%",
            SmsPurpose::Marketing,
            "ad-1",
        );
        let error = fx.sender.send(marketing).await.unwrap_err();
        assert_eq!(error.code(), "esms.marketing_unsupported");
        assert_eq!(error.kind(), ErrorKind::Validation);

        let foreign = SmsMessage::new(
            tenant,
            PhoneNumber::parse("+14155550100", "84").unwrap(),
            OTP_TEXT,
            SmsPurpose::Otp,
            "otp-1",
        );
        let error = fx.sender.send(foreign).await.unwrap_err();
        assert_eq!(error.code(), "esms.phone_unsupported");

        assert!(fx.server.take_requests().is_empty(), "không request nào tới eSMS");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ma_loi_cua_esms_thanh_code_on_dinh() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.account("one", "Baotrixemay").await;

        // Thân lỗi đúng như ví dụ trong tài liệu.
        let cases: [(u16, Value, &str, ErrorKind); 13] = [
            (200, json!({"CodeResult": "101", "CountRegenerate": 0, "ErrorMessage": "Authorize Failed"}), "esms.credentials_rejected", ErrorKind::Validation),
            (200, json!({"CodeResult": "103", "CountRegenerate": 0}), "esms.insufficient_balance", ErrorKind::Unavailable),
            (200, json!({"CodeResult": "104", "CountRegenerate": 0, "ErrorMessage": "Brand name code is not exist"}), "esms.brandname_rejected", ErrorKind::Validation),
            (200, json!({"CodeResult": "146", "CountRegenerate": 0, "ErrorMessage": "Sai template Brandname CSKH", "SMSID": "3f5e"}), "esms.template_rejected", ErrorKind::Validation),
            (200, json!({"CodeResult": "177"}), "esms.carrier_not_registered", ErrorKind::Validation),
            (200, json!({"CodeResult": "99"}), "esms.phone_rejected", ErrorKind::Validation),
            (200, json!({"CodeResult": 108}), "esms.phone_rejected", ErrorKind::Validation),
            (200, json!({"CodeResult": "201"}), "esms.content_blocked", ErrorKind::Validation),
            (200, json!({"CodeResult": "140"}), "esms.ip_not_allowed", ErrorKind::Validation),
            (200, json!({"CodeResult": "143"}), "esms.account_inactive", ErrorKind::Validation),
            (200, json!({"CodeResult": "118"}), "esms.request_rejected", ErrorKind::Validation),
            (200, json!({"error": "Invalid request"}), "esms.request_rejected", ErrorKind::Validation),
            (503, json!({"message": "down"}), "esms.unavailable", ErrorKind::Unavailable),
        ];
        for (status, body, _, _) in &cases {
            fx.server.reply("POST", SEND, *status, body.clone());
        }
        // Câu cuối: thân không phải JSON, coi là lỗi tạm thời.
        fx.server.reply_raw("POST", SEND, 200, "text/html", "<html>maintenance</html>");
        for (index, (_, body, code, kind)) in cases.into_iter().enumerate() {
            let error =
                fx.sender.send(fx.otp("one", &format!("ref-{index}")).await).await.unwrap_err();
            assert_eq!((error.code(), error.kind()), (code, kind), "{body}");
            // Câu lỗi không chứa key, số điện thoại hay nội dung tin.
            let message = error.message();
            for secret in ["api-key-of-one", "secret-key-of-one", "0901888484", "123456"] {
                assert!(!message.contains(secret), "{message}");
            }
        }

        let error = fx.sender.send(fx.otp("one", "ref-html").await).await.unwrap_err();
        assert_eq!(error.code(), "esms.unavailable");

        // Lỗi cuối của kết nối hiện cho người quản trị, không kèm secret.
        let listed = fx
            .app
            .get("/api/v1/kernel/settings?group=connection")
            .token("one-admin")
            .send()
            .await
            .unwrap()
            .text();
        assert!(!listed.contains("secret-key-of-one"), "{listed}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gui_lai_cung_reference_khong_thanh_tin_thu_hai() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.account("one", "Baotrixemay").await;

        fx.accepted("sms-1");
        // Bên gọi thử lại (mất câu trả lời): eSMS báo trùng RequestId, đúng như tài liệu.
        fx.server.reply(
            "POST",
            SEND,
            200,
            json!({"CodeResult": "124", "CountRegenerate": 0, "ErrorMessage": "Request exist 1", "SendStatus": "5"}),
        );
        fx.accepted("sms-2");
        fx.accepted("sms-3");
        fx.sender.send(fx.otp("one", "c82cd356-bf49-4113-9466-65a7f6359c96").await).await.unwrap();
        let again = fx
            .sender
            .send(fx.otp("one", "c82cd356-bf49-4113-9466-65a7f6359c96").await)
            .await
            .unwrap();
        assert_eq!(again.provider_reference, "request:c82cd356-bf49-4113-9466-65a7f6359c96");

        let seen = fx.server.take_requests();
        assert_eq!(seen.len(), 2);
        // Cùng reference → cùng RequestId: chính eSMS chặn tin thứ hai.
        assert_eq!(seen[0].json()["RequestId"], seen[1].json()["RequestId"]);

        // Reference dài quá 50 ký tự vẫn ra một RequestId hợp lệ và ổn định.
        let long = "r".repeat(90);
        fx.sender.send(fx.otp("one", &long).await).await.unwrap();
        fx.sender.send(fx.otp("one", &long).await).await.unwrap();
        let seen = fx.server.take_requests();
        let first = seen[0].json()["RequestId"].as_str().unwrap().to_owned();
        assert_eq!(first.len(), 50);
        assert_eq!(seen[1].json()["RequestId"], first.as_str());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn moi_truong_sandbox_cua_khach_gui_truong_sandbox() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        fx.account("one", "Baotrixemay").await;
        fx.account("two", "ShopHai").await;
        fx.set("one-admin", "esms.connection.sms.environment", "sandbox").await;
        fx.set("two-admin", "esms.connection.sms.environment", "production").await;

        fx.accepted("a");
        fx.accepted("b");
        fx.sender.send(fx.otp("one", "ref-1").await).await.unwrap();
        fx.sender.send(fx.otp("two", "ref-2").await).await.unwrap();
        let seen = fx.server.take_requests();
        assert_eq!(seen[0].json()["Sandbox"], "1");
        assert!(seen[1].json().get("Sandbox").is_none(), "khách `two` gửi thật");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thu_ket_noi_bang_ham_lay_so_du() {
    TestDatabase::run(async |test| {
        let fx = fixture(test).await;
        let test_url = "/api/v1/kernel/settings/esms.connection.sms.api_key/test";

        // Chưa cấu hình: kết quả nói rõ thiếu gì, không gọi eSMS.
        let answer = fx.app.post(test_url).token("one-admin").send().await.unwrap();
        assert_eq!(answer.status_code(), StatusCode::OK, "{}", answer.json());
        let body = answer.json();
        assert_eq!(body["data"]["connection"], "esms.connection.sms");
        assert_eq!(body["data"]["ok"], false);
        assert!(body["data"]["error"].as_str().unwrap().contains("esms.connection.sms.api_key"));
        assert!(fx.server.take_requests().is_empty());

        fx.account("one", "Baotrixemay").await;
        fx.server.reply(
            "POST",
            BALANCE,
            200,
            json!({"Balance": 36693, "CodeResponse": "100", "UserID": 9999}),
        );
        // Sai key: thân đúng như tài liệu.
        fx.server.reply(
            "POST",
            BALANCE,
            200,
            json!({"Balance": 0, "CodeResponse": "101", "ErrorMessage": "Authorize Failed"}),
        );
        let body = fx.app.post(test_url).token("one-admin").send().await.unwrap().json();
        assert_eq!(body["data"]["ok"], true, "{body}");
        assert!(!body.to_string().contains("36693"), "số dư không được trả ra");
        let seen = fx.server.take_requests();
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("POST", BALANCE));
        assert_eq!(
            seen[0].json(),
            json!({"ApiKey": "api-key-of-one", "SecretKey": "secret-key-of-one"})
        );

        let body = fx.app.post(test_url).token("one-admin").send().await.unwrap().json();
        assert_eq!(body["data"]["ok"], false);
        let error = body["data"]["error"].as_str().unwrap();
        assert!(error.contains("ApiKey") && !error.contains("secret-key-of-one"), "{error}");
    })
    .await
    .unwrap();
}
