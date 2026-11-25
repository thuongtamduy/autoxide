//! Mẫu câu SMS mang mã khách tự sửa (bước 70 P9, D-20261125-sms-templates): chưa đặt thì câu mặc định; đặt qua
//! cấu hình theo khách thì gửi đúng câu khách đặt; mẫu thiếu `{code}` hay có chỗ điền lạ bị từ chối. Cả SQLite
//! và Postgres.

use std::sync::{Arc, Mutex};

use autoxide_auth::api::{AuthExtensions, PhoneCodes};
use autoxide_kernel::error::Error;
use autoxide_kernel::sms::SmsSender;
use autoxide_testkit::{FakeSmsSender, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

use crate::tests::call;

#[derive(Default)]
struct Capture(Mutex<Option<Arc<dyn PhoneCodes>>>);

impl AuthExtensions for Capture {
    fn set_phone_codes(&self, codes: Arc<dyn PhoneCodes>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(codes);
        Ok(())
    }
}

async fn put_setting(app: &TestApp, name: &str, value: &str) -> (StatusCode, Value) {
    let answer = app
        .request(Method::PUT, &format!("/api/v1/kernel/settings/{name}"))
        .token("owner")
        .json(json!({"value": value}))
        .send()
        .await
        .unwrap();
    (answer.status(), answer.json())
}

#[tokio::test]
async fn khach_tu_sua_mau_sms_mang_ma() {
    TestDatabase::run(async |test| {
        let sms = Arc::new(FakeSmsSender::new());
        let sender: Arc<dyn SmsSender> = sms.clone();
        let capture = Arc::new(Capture::default());
        let app = TestApp::builder(test)
            .user("owner", 1, &["kernel.settings.manage"])
            .user("alice", 2, &[])
            .provide::<dyn SmsSender>(sender)
            .provide::<dyn AuthExtensions>(capture.clone())
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let codes = capture.0.lock().unwrap().clone().unwrap();
        let tenant = app.default_tenant();

        // Chưa đặt: câu mặc định như trước.
        let (status, sent) = call(
            &app,
            "POST",
            "/me/phone-verifications",
            "alice",
            Some(json!({"phone": "0912345678"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{sent}");
        let text = sms.last_text("+84912345678").unwrap();
        assert!(text.starts_with("Ma xac thuc cua ban: "), "{text}");
        codes.send(&tenant, "0912000001", None).await.unwrap();
        let text = sms.last_text("+84912000001").unwrap();
        assert!(text.starts_with("Ma dang nhap cua ban: "), "{text}");

        // Mẫu sai bị từ chối, lỗi ở ô `value`.
        let (status, problem) =
            put_setting(&app, "compliance.sms_template.sign_in", "[SHOP] Ma cua ban").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"][0]["field"], "value", "{problem}");
        assert_eq!(problem["errors"][0]["code"], "placeholder_code", "{problem}");
        let (status, problem) =
            put_setting(&app, "compliance.sms_template.sign_in", "Ma {code} don {number}").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"][0]["code"], "placeholder_unknown", "{problem}");

        // Mẫu đúng: gửi đúng câu khách đặt.
        let (status, saved) = put_setting(
            &app,
            "compliance.sms_template.sign_in",
            "[SHOP] {code} la ma dang nhap, hieu luc {minutes} phut",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let (status, saved) =
            put_setting(&app, "compliance.sms_template.phone_verification", "[SHOP] OTP: {code}")
                .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        codes.send(&tenant, "0912000002", None).await.unwrap();
        let text = sms.last_text("+84912000002").unwrap();
        let code = text.split(' ').nth(1).unwrap();
        assert_eq!(code.len(), 6, "{text}");
        assert_eq!(text, format!("[SHOP] {code} la ma dang nhap, hieu luc 5 phut"));
        let (status, sent) = call(
            &app,
            "POST",
            "/me/phone-verifications",
            "alice",
            Some(json!({"phone": "0912000003"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{sent}");
        let text = sms.last_text("+84912000003").unwrap();
        assert!(
            text.starts_with("[SHOP] OTP: ") && text.len() == "[SHOP] OTP: ".len() + 6,
            "{text}"
        );

        // Danh sách theo nhóm cho màn hình "mẫu SMS".
        let answer = app
            .request(Method::GET, "/api/v1/kernel/settings?group=sms_template")
            .token("owner")
            .send()
            .await
            .unwrap();
        let names: Vec<String> = answer.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.contains(&"compliance.sms_template.sign_in".to_owned()), "{names:?}");
        assert!(
            names.contains(&"compliance.sms_template.phone_verification".to_owned()),
            "{names:?}"
        );
    })
    .await
    .unwrap();
}

/// Cấu hình theo khách không đọc được (database bận…).
struct BrokenSettings;

#[async_trait::async_trait]
impl autoxide_kernel::tenancy::TenantSettings for BrokenSettings {
    async fn get(
        &self,
        _tenant: &autoxide_kernel::tenancy::TenantContext,
        _setting: &autoxide_kernel::tenancy::TenantSetting,
    ) -> Result<Option<autoxide_kernel::tenancy::SettingValue>, Error> {
        Err(Error::internal("the database did not answer"))
    }
}

/// Đọc mẫu lỗi sau khi đã lưu mã: không lỗi, rơi về câu mặc định — mã đã lưu vẫn được gửi.
#[tokio::test]
async fn doc_mau_loi_thi_gui_cau_mac_dinh() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).module(crate::module()).build().await.unwrap();
        let tenant = app.default_tenant();
        let text = autoxide_kernel::sms::tenant_sms_text(
            &BrokenSettings,
            &tenant,
            &crate::sms_templates::SIGN_IN,
            &["code", "minutes"],
            &[("code", "123456"), ("minutes", "5")],
        )
        .await;
        assert_eq!(text, None);
    })
    .await
    .unwrap();
}
