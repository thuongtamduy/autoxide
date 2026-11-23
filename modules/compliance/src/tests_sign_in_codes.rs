//! Mã đăng nhập bằng số (bước 47): `compliance` cắm bộ mã vào `auth` qua `AuthExtensions`; mã dùng
//! một lần, sai thì đếm, hai lần nhập cùng lúc chỉ một bên được; gắn số vào tài khoản theo quy tắc số
//! tài khoản mỗi số. Cả SQLite và Postgres.

use std::sync::{Arc, Mutex};

use autoxide_auth::api::{AuthExtensions, PhoneCodes};
use autoxide_kernel::error::Error;
use autoxide_kernel::sms::SmsSender;
use autoxide_kernel::tenancy::{DEFAULT_TENANT, ScopeExt};
use autoxide_testkit::{FakeSmsSender, TestApp, TestDatabase};
use uuid::Uuid;

use crate::api::AccountVerification;

/// `auth` giả: giữ bộ mã `compliance` cắm vào.
#[derive(Default)]
struct Capture(Mutex<Option<Arc<dyn PhoneCodes>>>);

impl AuthExtensions for Capture {
    fn set_phone_codes(&self, codes: Arc<dyn PhoneCodes>) -> Result<(), Error> {
        *self.0.lock().unwrap() = Some(codes);
        Ok(())
    }
}

async fn app(test: &TestDatabase) -> (TestApp, Arc<dyn PhoneCodes>, Arc<FakeSmsSender>) {
    app_with(test, &[]).await
}

async fn app_with(
    test: &TestDatabase,
    settings: &[(&str, &str)],
) -> (TestApp, Arc<dyn PhoneCodes>, Arc<FakeSmsSender>) {
    let sms = Arc::new(FakeSmsSender::new());
    let capture = Arc::new(Capture::default());
    let sender: Arc<dyn SmsSender> = sms.clone();
    let mut builder = TestApp::builder(test);
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    let app = builder
        .provide::<dyn SmsSender>(sender)
        .provide::<dyn AuthExtensions>(capture.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    let codes = capture.0.lock().unwrap().clone().expect("compliance plugged its codes in");
    (app, codes, sms)
}

fn code_in(sms: &FakeSmsSender, phone: &str) -> String {
    let text = sms.last_text(phone).unwrap();
    text.split(|c: char| !c.is_ascii_digit()).find(|part| part.len() == 6).unwrap().to_owned()
}

#[tokio::test]
async fn ma_dang_nhap_dung_mot_lan_va_gan_so_theo_quy_tac() {
    TestDatabase::run(async |test| {
        let (app, codes, sms) = app(test).await;
        let tenant = app.default_tenant();
        let sent = codes.send(&tenant, "0912 345 678", None).await.unwrap();
        let code = code_in(&sms, "+84912345678");
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let error = codes.verify(&tenant, sent.id, wrong).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_invalid");
        let phone = codes.verify(&tenant, sent.id, &code).await.unwrap();
        assert_eq!(phone.as_str(), "+84912345678");
        // Dùng một lần.
        let error = codes.verify(&tenant, sent.id, &code).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_invalid");
        // Mã lạ.
        let error = codes.verify(&tenant, Uuid::now_v7(), &code).await.unwrap_err();
        assert_eq!(error.code(), "compliance.challenge_not_found");

        // Gắn số vào tài khoản: module khác thấy qua `AccountVerification`.
        let accounts = app.api::<dyn AccountVerification>().unwrap();
        let (lan, minh) = (Uuid::from_u128(21), Uuid::from_u128(22));
        codes.record(&tenant, lan, &phone).await.unwrap();
        let verified = accounts.verified_phone(DEFAULT_TENANT, lan).await.unwrap();
        assert_eq!(
            verified.map(|phone| phone.as_str().to_owned()).as_deref(),
            Some("+84912345678")
        );
        // Quy tắc mặc định một tài khoản mỗi số: tài khoản thứ hai không được gắn, không lỗi.
        codes.record(&tenant, minh, &phone).await.unwrap();
        assert!(accounts.verified_phone(DEFAULT_TENANT, minh).await.unwrap().is_none());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_nhap_ma_cung_luc_chi_mot_ben_duoc() {
    TestDatabase::run(async |test| {
        let (app, codes, sms) = app(test).await;
        let tenant = app.default_tenant();
        let sent = codes.send(&tenant, "0987654321", None).await.unwrap();
        let code = code_in(&sms, "+84987654321");
        let (left, right) = tokio::join!(
            codes.verify(&tenant, sent.id, &code),
            codes.verify(&tenant, sent.id, &code),
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1, "{left:?} {right:?}");
    })
    .await
    .unwrap();
}

/// Hạn mức sai mỗi ngày của một số: cộng mọi mã của số trong 24 giờ — xin mã mới không mở thêm
/// lượt đoán; đủ hạn mức thì không kiểm, không gửi mã nữa, tới khi lần sai cũ ra khỏi 24 giờ.
#[tokio::test]
async fn sai_qua_han_muc_moi_ngay_thi_khoa_so() {
    TestDatabase::run(async |test| {
        let settings = [
            ("AUTOXIDE_COMPLIANCE_OTP_WRONG_PER_PHONE_PER_DAY", "4"),
            ("AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR", "100"),
        ];
        let (app, codes, sms) = app_with(test, &settings).await;
        let tenant = app.default_tenant();
        let wrong_for = |code: &str| if code == "000000" { "111111" } else { "000000" }.to_owned();

        let first = codes.send(&tenant, "0912 000 111", None).await.unwrap();
        let wrong = wrong_for(&code_in(&sms, "+84912000111"));
        for _ in 0..3 {
            let error = codes.verify(&tenant, first.id, &wrong).await.unwrap_err();
            assert_eq!(error.code(), "compliance.otp_invalid");
        }
        // Mã mới: chỉ còn một lượt sai trong ngày.
        let second = codes.send(&tenant, "0912000111", None).await.unwrap();
        let code = code_in(&sms, "+84912000111");
        let error = codes.verify(&tenant, second.id, &wrong_for(&code)).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_invalid");
        // Đủ 4 lần sai: mã đúng cũng không được kiểm, không gửi thêm mã.
        let error = codes.verify(&tenant, second.id, &code).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_daily_limit");
        assert!(error.retry_after().is_some_and(|seconds| seconds > 0));
        let sent_before = sms.sent.lock().unwrap().len();
        let error = codes.send(&tenant, "0912000111", None).await.unwrap_err();
        assert_eq!(error.code(), "compliance.otp_daily_limit");
        assert_eq!(
            sms.sent.lock().unwrap().len(),
            sent_before,
            "không gửi SMS khi số đang bị khoá"
        );
        // Số khác không bị ảnh hưởng.
        codes.send(&tenant, "0912000222", None).await.unwrap();

        // Qua 24 giờ (dời mốc tạo của các mã): gửi và kiểm lại được.
        {
            use sea_orm::sea_query::Expr;
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            let past =
                autoxide_kernel::time::Timestamp::now() - jiff::SignedDuration::from_hours(25);
            crate::entity::phone_challenge::Entity::update_many()
                .scoped(&tenant)
                .col_expr(crate::entity::phone_challenge::Column::CreatedAt, Expr::value(past))
                .filter(crate::entity::phone_challenge::Column::Phone.eq("+84912000111"))
                .exec(app.database().write())
                .await
                .unwrap();
        }
        let third = codes.send(&tenant, "0912000111", None).await.unwrap();
        let code = code_in(&sms, "+84912000111");
        codes.verify(&tenant, third.id, &code).await.unwrap();
    })
    .await
    .unwrap();
}
