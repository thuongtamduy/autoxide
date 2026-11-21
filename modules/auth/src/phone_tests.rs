//! Đăng nhập / tạo tài khoản người mua bằng số điện thoại + mã SMS (bước 47): tài khoản chỉ có số,
//! đăng nhập lại cùng tài khoản, hai lần tạo đồng thời một tài khoản, xác nhận lại bằng mã. Bộ mã giả
//! cắm qua `AuthExtensions` như `compliance` làm. Cả SQLite và Postgres.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AuthExtensions, PhoneCodeSent, PhoneCodes};
use crate::http::test_support::{app, call};

const CODE: &str = "123456";

/// Bộ mã giả: mọi mã là `123456`, dùng một lần; ghi lại số được gắn vào tài khoản nào.
#[derive(Default)]
struct FakeCodes {
    sent: Mutex<HashMap<Uuid, (PhoneNumber, bool)>>,
    recorded: Mutex<Vec<(Uuid, String)>>,
}

#[async_trait::async_trait]
impl PhoneCodes for FakeCodes {
    async fn send(
        &self,
        _: &TenantContext,
        phone: &str,
        _: Option<std::net::IpAddr>,
    ) -> Result<PhoneCodeSent, Error> {
        let phone = PhoneNumber::parse(phone, "84").ok_or_else(|| {
            Error::validation(vec![
                FieldError::new("phone", "format", "is not valid: {reason}")
                    .with_param("reason", "not a phone number"),
            ])
        })?;
        let id = Uuid::now_v7();
        self.sent.lock().unwrap().insert(id, (phone, false));
        Ok(PhoneCodeSent { id, expires_at: Timestamp::now() + SignedDuration::from_mins(5) })
    }
    async fn verify(&self, _: &TenantContext, id: Uuid, code: &str) -> Result<PhoneNumber, Error> {
        let mut sent = self.sent.lock().unwrap();
        let entry = sent.get_mut(&id).ok_or_else(|| Error::internal("no such code"))?;
        if code != CODE || entry.1 {
            return Err(Error::validation(vec![
                FieldError::new("code", "invalid", "is not valid: {reason}")
                    .with_param("reason", "wrong code"),
            ]));
        }
        entry.1 = true;
        Ok(entry.0.clone())
    }
    async fn record(
        &self,
        _: &TenantContext,
        user_id: Uuid,
        phone: &PhoneNumber,
    ) -> Result<(), Error> {
        self.recorded.lock().unwrap().push((user_id, phone.as_str().to_owned()));
        Ok(())
    }
}

async fn plugged(test: &TestDatabase) -> (TestApp, Arc<FakeCodes>) {
    let (app, _) = app(test).await;
    let codes = Arc::new(FakeCodes::default());
    app.api::<dyn AuthExtensions>().unwrap().set_phone_codes(codes.clone()).unwrap();
    (app, codes)
}

async fn code_for(app: &TestApp, phone: &str) -> String {
    let (status, _, sent) =
        call(app, "POST", "/phone/codes", None, Some(json!({"phone": phone}))).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{sent}");
    sent["data"]["challenge_id"].as_str().unwrap().to_owned()
}

async fn login(app: &TestApp, phone: &str, name: Option<&str>) -> (StatusCode, Value) {
    let challenge = code_for(app, phone).await;
    let body = json!({"challenge_id": challenge, "code": CODE, "name": name});
    let (status, _, answer) = call(app, "POST", "/phone/login", None, Some(body)).await;
    (status, answer)
}

async fn me(app: &TestApp, answer: &Value) -> Value {
    let bearer = format!("Bearer {}", answer["data"]["access_token"].as_str().unwrap());
    let (status, _, me) = call(app, "GET", "/me", Some(&bearer), None).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    me["data"].clone()
}

#[tokio::test]
async fn so_moi_tao_tai_khoan_chi_co_so_roi_dang_nhap_lai() {
    TestDatabase::run(async |test| {
        let (app, codes) = plugged(test).await;
        let (status, created) = login(&app, "0912345678", Some("Lan")).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let first = me(&app, &created).await;
        assert_eq!((first["email"].as_str(), first["name"].as_str()), (Some(""), Some("Lan")));
        assert_eq!(
            (first["has_password"].as_bool(), first["permissions"].clone()),
            (Some(false), json!([])),
            "{first}"
        );
        // Lần sau: cùng tài khoản, 200; tên gửi kèm bị bỏ qua.
        let (status, again) = login(&app, "+84912345678", Some("Khác")).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        let second = me(&app, &again).await;
        assert_eq!(
            (second["id"].clone(), second["name"].as_str()),
            (first["id"].clone(), Some("Lan"))
        );
        // Số được gắn vào tài khoản (cho `compliance`) mỗi lần.
        let recorded = codes.recorded.lock().unwrap().clone();
        assert_eq!(recorded.len(), 2);
        assert!(recorded.iter().all(|(_, phone)| phone == "+84912345678"));
        // Số khác: tài khoản khác, cũng không email (unique email bỏ qua email rỗng); không tên thì
        // tên là số đã che.
        let (status, other) = login(&app, "0987654321", None).await;
        assert_eq!(status, StatusCode::CREATED, "{other}");
        let other = me(&app, &other).await;
        assert_ne!(other["id"], first["id"]);
        assert!(other["name"].as_str().unwrap().ends_with("321"), "{other}");
        // Mã đã dùng không dùng lại được.
        let challenge = code_for(&app, "0912345678").await;
        let body = json!({"challenge_id": challenge, "code": CODE});
        let (status, _, _) = call(&app, "POST", "/phone/login", None, Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, reused) = call(&app, "POST", "/phone/login", None, Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{reused}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chua_cam_bo_ma_thi_503() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let body = json!({"phone": "0912345678"});
        let (status, _, answer) = call(&app, "POST", "/phone/codes", None, Some(body)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
        assert_eq!(answer["code"], "auth.phone_sign_in_unavailable");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_tao_dong_thoi_cung_so_chi_mot_tai_khoan() {
    TestDatabase::run(async |test| {
        let (app, _) = plugged(test).await;
        let first = code_for(&app, "0912000111").await;
        let second = code_for(&app, "0912000111").await;
        let one = json!({"challenge_id": first, "code": CODE});
        let two = json!({"challenge_id": second, "code": CODE});
        let ((a, _, left), (b, _, right)) = tokio::join!(
            call(&app, "POST", "/phone/login", None, Some(one)),
            call(&app, "POST", "/phone/login", None, Some(two)),
        );
        assert!(a.is_success() && b.is_success(), "{left} {right}");
        assert_eq!(me(&app, &left).await["id"], me(&app, &right).await["id"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xac_nhan_lai_bang_ma_cua_dung_so() {
    TestDatabase::run(async |test| {
        let (app, _) = plugged(test).await;
        let (_, signed_in) = login(&app, "0912345678", Some("Lan")).await;
        let bearer = format!("Bearer {}", signed_in["data"]["access_token"].as_str().unwrap());
        // Không có email: không xác nhận qua mail được.
        let body = json!({"method": "email_link"});
        let (status, _, answer) = call(&app, "POST", "/me/reauth", Some(&bearer), Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        // Mã gửi tới số khác: từ chối.
        let other = code_for(&app, "0987654321").await;
        let body = json!({"method": "phone_code", "phone_challenge_id": other, "phone_code": CODE});
        let (status, _, answer) = call(&app, "POST", "/me/reauth", Some(&bearer), Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{answer}");
        // Mã gửi tới đúng số: được token xác nhận lại.
        let own = code_for(&app, "0912345678").await;
        let body = json!({"method": "phone_code", "phone_challenge_id": own, "phone_code": CODE});
        let (status, _, answer) = call(&app, "POST", "/me/reauth", Some(&bearer), Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert!(answer["data"]["reauth_token"].is_string());
    })
    .await
    .unwrap();
}
