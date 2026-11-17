//! Test xác thực tài khoản đăng nội dung (NĐ 147/2024, D-356) trên cả SQLite và Postgres: mã một lần
//! qua SMS giả, số tài khoản mỗi số, số lần sai, hết hạn, giới hạn số mã, hai lần xác nhận cùng lúc
//! chỉ một bên thắng, cổng đăng nội dung theo chính sách của khách, mức định danh từ KYC.

use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::identity::IdentityVerifier;
use autoxide_kernel::sms::{SmsMessage, SmsReceipt, SmsSender};
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_testkit::{FakeIdentityVerifier, FakeSmsSender, TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::sea_query::Expr;
use sea_orm::{EntityTrait, PaginatorTrait};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{AccountVerification, PostingGate, PostingSurface, VerificationLevel};
use crate::entity::phone_challenge;
use crate::tests::{call, callback, run_jobs, start};

const ALICE_PHONE: &str = "+84912345678";

async fn app_with(
    test: &TestDatabase,
    sms: Option<Arc<dyn SmsSender>>,
    settings: &[(&str, &str)],
) -> (TestApp, Arc<FakeIdentityVerifier>) {
    let verifier = Arc::new(FakeIdentityVerifier::new());
    let mut builder = TestApp::builder(test)
        .user("admin", 1, &["compliance.policies.manage", "compliance.kyc.read"])
        .user("alice", 2, &[])
        .user("bob", 3, &[])
        .provide::<dyn IdentityVerifier>(verifier.clone());
    for (name, value) in settings {
        builder = builder.setting(name, value);
    }
    if let Some(sms) = sms {
        builder = builder.provide::<dyn SmsSender>(sms);
    }
    (builder.module(crate::module()).build().await.unwrap(), verifier)
}

async fn app(test: &TestDatabase) -> (TestApp, Arc<FakeSmsSender>) {
    let sms = Arc::new(FakeSmsSender::new());
    let (app, _) = app_with(test, Some(sms.clone()), &[]).await;
    (app, sms)
}

/// Mã 6 số trong tin cuối gửi tới `phone`.
fn code_in(sms: &FakeSmsSender, phone: &str) -> String {
    let text = sms.last_text(phone).unwrap();
    text.split(|c: char| !c.is_ascii_digit()).find(|part| part.len() == 6).unwrap().to_owned()
}

/// Xin mã cho `phone`; trả ID lượt xác thực.
async fn ask(app: &TestApp, who: &str, phone: &str) -> String {
    let (status, sent) =
        call(app, "POST", "/me/phone-verifications", who, Some(json!({"phone": phone}))).await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");
    sent["data"]["id"].as_str().unwrap().to_owned()
}

async fn confirm(app: &TestApp, who: &str, id: &str, code: &str) -> (StatusCode, Value) {
    let uri = format!("/me/phone-verifications/{id}/confirm");
    call(app, "POST", &uri, who, Some(json!({"code": code}))).await
}

#[tokio::test]
async fn xac_thuc_so_dien_thoai_bang_ma_mot_lan() {
    TestDatabase::run(async |test| {
        let (app, sms) = app(test).await;
        let (_, mine) = call(&app, "GET", "/me/verification", "alice", None).await;
        assert_eq!(mine["data"]["level"], "none", "{mine}");

        let id = ask(&app, "alice", "0912 345 678").await;
        let code = code_in(&sms, ALICE_PHONE);
        // Mã không bao giờ nằm trong bảng nguyên văn.
        let row = phone_challenge::Entity::find_by_id(id.parse::<Uuid>().unwrap())
            .one(app.database().read())
            .await
            .unwrap()
            .unwrap();
        assert!(!row.code_hash.contains(&code));
        let (status, seen) =
            call(&app, "GET", &format!("/me/phone-verifications/{id}"), "alice", None).await;
        assert_eq!(status, StatusCode::OK, "{seen}");
        assert_eq!(seen["data"]["phone"], "+8491****678");
        // Người khác không thấy lượt của Alice.
        let (status, _) =
            call(&app, "GET", &format!("/me/phone-verifications/{id}"), "bob", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = confirm(&app, "bob", &id, &code).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let wrong = if code == "000000" { "111111" } else { "000000" };
        let (status, problem) = confirm(&app, "alice", &id, wrong).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "compliance.otp_invalid");

        let (status, done) = confirm(&app, "alice", &id, &code).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["data"]["level"], "phone");
        assert_eq!(done["data"]["phone"], "+8491****678");
        // Dùng một lần.
        let (status, again) = confirm(&app, "alice", &id, &code).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{again}");

        let verification = app.api::<dyn AccountVerification>().unwrap();
        let alice = Uuid::from_u128(2);
        assert_eq!(
            verification.level(DEFAULT_TENANT, alice).await.unwrap(),
            VerificationLevel::Phone
        );
        let phone = verification.verified_phone(DEFAULT_TENANT, alice).await.unwrap().unwrap();
        assert_eq!(phone.as_str(), ALICE_PHONE);

        // Gỡ số: về `none`; gọi lại vô hại.
        let (status, _) = call(&app, "DELETE", "/me/phone", "alice", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call(&app, "DELETE", "/me/phone", "alice", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert_eq!(
            verification.level(DEFAULT_TENANT, alice).await.unwrap(),
            VerificationLevel::None
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_khong_hop_le_hay_khong_co_nha_cung_cap_sms() {
    TestDatabase::run(async |test| {
        let (app, _) = app_with(test, None, &[]).await;
        let (status, problem) =
            call(&app, "POST", "/me/phone-verifications", "alice", Some(json!({"phone": "abc"})))
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"][0]["field"], "phone", "{problem}");
        // Số cố định và số nước ngoài: không phải số di động Việt Nam (quy tắc `posting.phone_countries`).
        for phone in ["024 3825 1234", "+1 415 555 0100"] {
            let (status, problem) = call(
                &app,
                "POST",
                "/me/phone-verifications",
                "alice",
                Some(json!({"phone": phone})),
            )
            .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{phone}");
            assert_eq!(problem["code"], "compliance.phone_not_allowed");
        }
        // Chưa cấu hình gửi SMS: lỗi rõ, không panic.
        let (status, problem) = call(
            &app,
            "POST",
            "/me/phone-verifications",
            "alice",
            Some(json!({"phone": ALICE_PHONE})),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(problem["code"], "sms_disabled");
    })
    .await
    .unwrap();
}

/// Nhà cung cấp SMS luôn lỗi.
struct BrokenSms;

#[async_trait::async_trait]
impl SmsSender for BrokenSms {
    fn provider(&self) -> &'static str {
        "broken"
    }

    async fn send(&self, _message: SmsMessage) -> Result<SmsReceipt, Error> {
        // Nhà cung cấp hỏng trả lỗi tạm thời như khi chưa cấu hình.
        Err(autoxide_kernel::sms::sms_disabled())
    }
}

#[tokio::test]
async fn gui_sms_loi_thi_xoa_ma() {
    TestDatabase::run(async |test| {
        let (app, _) = app_with(test, Some(Arc::new(BrokenSms)), &[]).await;
        let (status, problem) = call(
            &app,
            "POST",
            "/me/phone-verifications",
            "alice",
            Some(json!({"phone": ALICE_PHONE})),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{problem}");
        let left = phone_challenge::Entity::find().count(app.database().read()).await.unwrap();
        assert_eq!(left, 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn mot_so_cho_mot_tai_khoan_theo_chinh_sach() {
    TestDatabase::run(async |test| {
        let (app, sms) = app(test).await;
        let id = ask(&app, "alice", ALICE_PHONE).await;
        let (status, _) = confirm(&app, "alice", &id, &code_in(&sms, ALICE_PHONE)).await;
        assert_eq!(status, StatusCode::OK);

        let (status, problem) = call(
            &app,
            "POST",
            "/me/phone-verifications",
            "bob",
            Some(json!({"phone": ALICE_PHONE})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "compliance.phone_in_use");

        // Khách cho hai tài khoản mỗi số.
        let (status, _) = call(
            &app,
            "PUT",
            "/policies/posting.max_accounts_per_phone",
            "admin",
            Some(json!({"value": 2})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let id = ask(&app, "bob", ALICE_PHONE).await;
        let (status, done) = confirm(&app, "bob", &id, &code_in(&sms, ALICE_PHONE)).await;
        assert_eq!(status, StatusCode::OK, "{done}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sai_qua_so_lan_het_han_va_gioi_han_so_ma() {
    TestDatabase::run(async |test| {
        let sms = Arc::new(FakeSmsSender::new());
        let (app, _) = app_with(
            test,
            Some(sms.clone()),
            &[
                ("AUTOXIDE_COMPLIANCE_OTP_MAX_ATTEMPTS", "2"),
                ("AUTOXIDE_COMPLIANCE_OTP_PER_PHONE_PER_HOUR", "3"),
            ],
        )
        .await;
        let id = ask(&app, "alice", ALICE_PHONE).await;
        let code = code_in(&sms, ALICE_PHONE);
        let wrong = if code == "000000" { "111111" } else { "000000" };
        let (status, _) = confirm(&app, "alice", &id, wrong).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, problem) = confirm(&app, "alice", &id, wrong).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{problem}");
        assert_eq!(problem["code"], "compliance.otp_attempts_exceeded");
        // Đã quá số lần: mã đúng cũng không nhận.
        let (status, _) = confirm(&app, "alice", &id, &code).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

        // Hết hạn.
        let id = ask(&app, "alice", ALICE_PHONE).await;
        let code = code_in(&sms, ALICE_PHONE);
        phone_challenge::Entity::update_many()
            .col_expr(
                phone_challenge::Column::ExpiresAt,
                Expr::value(chrono::Utc::now() - chrono::Duration::minutes(1)),
            )
            .exec(app.database().write())
            .await
            .unwrap();
        let (status, problem) = confirm(&app, "alice", &id, &code).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["code"], "compliance.otp_invalid");

        // Mã thứ tư trong giờ cho cùng số: bị giới hạn.
        ask(&app, "alice", ALICE_PHONE).await;
        let (status, problem) = call(
            &app,
            "POST",
            "/me/phone-verifications",
            "alice",
            Some(json!({"phone": ALICE_PHONE})),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{problem}");
        assert_eq!(problem["code"], "compliance.otp_throttled");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_lan_xac_nhan_cung_luc_chi_mot_ben_thang() {
    TestDatabase::run(async |test| {
        let (app, sms) = app(test).await;
        let id = ask(&app, "alice", ALICE_PHONE).await;
        let code = code_in(&sms, ALICE_PHONE);
        let (first, second) =
            tokio::join!(confirm(&app, "alice", &id, &code), confirm(&app, "alice", &id, &code));
        let mut statuses = [first.0, second.0];
        statuses.sort();
        assert_eq!(
            statuses,
            [StatusCode::OK, StatusCode::UNPROCESSABLE_ENTITY],
            "{} {}",
            first.1,
            second.1
        );

        // Hai người cùng nhận một số (mỗi số một tài khoản): chỉ một người thắng.
        let bob_id = ask(&app, "bob", "0987 654 321").await;
        let bob_code = code_in(&sms, "+84987654321");
        let (status, _) = call(&app, "DELETE", "/me/phone", "alice", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let alice_id = ask(&app, "alice", "0987 654 321").await;
        let alice_code = code_in(&sms, "+84987654321");
        let (a, b) = tokio::join!(
            confirm(&app, "alice", &alice_id, &alice_code),
            confirm(&app, "bob", &bob_id, &bob_code)
        );
        let mut statuses = [a.0, b.0];
        statuses.sort();
        assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT], "{} {}", a.1, b.1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cong_dang_noi_dung_theo_chinh_sach_va_dinh_danh() {
    TestDatabase::run(async |test| {
        let sms = Arc::new(FakeSmsSender::new());
        let (app, verifier) = app_with(test, Some(sms.clone()), &[]).await;
        let gate = app.api::<dyn PostingGate>().unwrap();
        let alice =
            autoxide_kernel::access::Principal::new(Uuid::from_u128(2), Vec::<String>::new());
        let bob = autoxide_kernel::access::Principal::new(Uuid::from_u128(3), Vec::<String>::new());
        // Mặc định `none`: ai cũng đăng được, kể cả khách vãng lai.
        gate.check(DEFAULT_TENANT, None, PostingSurface::Reviews).await.unwrap();

        let set = |key: &'static str, value: &'static str| {
            let app = &app;
            async move {
                let (status, body) = call(
                    app,
                    "PUT",
                    &format!("/policies/{key}"),
                    "admin",
                    Some(json!({"value": value})),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{body}");
            }
        };
        set("posting.reviews", "phone").await;
        let error = gate.check(DEFAULT_TENANT, None, PostingSurface::Reviews).await.unwrap_err();
        assert_eq!(error.code(), "compliance.verification_required");
        let error =
            gate.check(DEFAULT_TENANT, Some(&alice), PostingSurface::Reviews).await.unwrap_err();
        assert_eq!(error.code(), "compliance.verification_required");
        let id = ask(&app, "alice", ALICE_PHONE).await;
        confirm(&app, "alice", &id, &code_in(&sms, ALICE_PHONE)).await;
        gate.check(DEFAULT_TENANT, Some(&alice), PostingSurface::Reviews).await.unwrap();
        // Chỗ khác vẫn `none`.
        gate.check(DEFAULT_TENANT, Some(&bob), PostingSurface::Livestream).await.unwrap();

        // `identity`: số điện thoại chưa đủ; định danh qua KYC thì đủ (bao cả số điện thoại).
        set("posting.livestream", "identity").await;
        let error =
            gate.check(DEFAULT_TENANT, Some(&alice), PostingSurface::Livestream).await.unwrap_err();
        assert_eq!(error.code(), "compliance.verification_required");
        let (_, reference) = start(&app, "bob").await;
        verifier.complete(&reference, true, "TRAN VAN B", "001099054321");
        assert_eq!(callback(&app, &reference).await, StatusCode::NO_CONTENT);
        run_jobs(&app).await;
        gate.check(DEFAULT_TENANT, Some(&bob), PostingSurface::Livestream).await.unwrap();
        gate.check(DEFAULT_TENANT, Some(&bob), PostingSurface::Reviews).await.unwrap();
        let (_, mine) = call(&app, "GET", "/me/verification", "bob", None).await;
        assert_eq!(mine["data"]["level"], "identity", "{mine}");
        assert!(mine["data"]["identity_verified_at"].is_string());
    })
    .await
    .unwrap();
}
