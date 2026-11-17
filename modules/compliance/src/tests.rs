//! Test của module `compliance` qua REST và trait công bố (cả SQLite và Postgres): định danh qua nhà
//! cung cấp giả (callback có chữ ký → job → `verified`), trùng giấy tờ phải duyệt tay, duyệt chỉ bởi
//! người đăng nhập, cổng KYC theo quy tắc có ngày hiệu lực, văn bản có phiên bản + chấp nhận lại, sổ
//! đồng ý (rút ở khoá rộng hơn thắng), nhập quy tắc chỉ thêm.

use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::identity::IdentityVerifier;
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_testkit::{FakeIdentityVerifier, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use chrono::{TimeZone, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{
    AcceptanceGate, Activity, ComplianceRules, ConsentEvidence, ConsentKey, ConsentLedger,
    ConsentSubject, KycGate, Subject,
};

const ALL: &[&str] = &[
    "compliance.kyc.read",
    "compliance.kyc.submit",
    "compliance.kyc.review",
    "compliance.documents.read",
    "compliance.documents.manage",
    "compliance.policies.manage",
    "compliance.rules.read",
];

async fn app(test: &TestDatabase) -> (TestApp, Arc<FakeIdentityVerifier>) {
    let verifier = Arc::new(FakeIdentityVerifier::new());
    let app = TestApp::builder(test)
        .user("admin", 1, ALL)
        .user("alice", 2, &[])
        .user("bob", 3, &[])
        .user("staff", 5, &["compliance.rules.read", "compliance.rules.manage"])
        .principal(
            "robot",
            Principal::new(Uuid::from_u128(4), ALL.iter().map(|name| (*name).to_owned()))
                .via_api_key(Uuid::from_u128(40)),
        )
        .provide::<dyn IdentityVerifier>(verifier.clone())
        .module(crate::module())
        .build()
        .await
        .unwrap();
    (app, verifier)
}

pub(crate) async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = app.request(method, &format!("/api/v1/compliance{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

pub(crate) async fn run_jobs(app: &TestApp) {
    // Đủ chỗ cho mọi lịch của module chạy trong một lượt.
    app.modules().worker(8).unwrap().blocked_delay(Duration::ZERO).run_once().await.unwrap();
}

/// Mở hồ sơ của `who`, bắt đầu lượt xác thực; trả (id hồ sơ, mã ở nhà cung cấp).
pub(crate) async fn start(app: &TestApp, who: &str) -> (String, String) {
    let (status, opened) = call(app, "POST", "/me/kyc", who, Some(json!({}))).await;
    assert!(status.is_success(), "{opened}");
    let (status, session) =
        call(app, "POST", "/me/kyc/sessions", who, Some(json!({"method": "vneid"}))).await;
    assert!(status.is_success(), "{session}");
    assert_eq!(session["data"]["kind"], "redirect");
    let attempt: Uuid = session["data"]["attempt_id"].as_str().unwrap().parse().unwrap();
    (
        session["data"]["profile_id"].as_str().unwrap().to_owned(),
        FakeIdentityVerifier::reference(attempt),
    )
}

pub(crate) async fn callback(app: &TestApp, reference: &str) -> StatusCode {
    let (headers, body) = FakeIdentityVerifier::callback(reference);
    let mut request = app.post("/api/v1/compliance/kyc/callbacks/fake").bytes(body);
    for (name, value) in &headers {
        request = request.header(name, value);
    }
    request.header("content-type", "application/json").send().await.unwrap().status()
}

#[tokio::test]
async fn dinh_danh_qua_nha_cung_cap_roi_cong_kyc_mo() {
    TestDatabase::run(async |test| {
        let (app, verifier) = app(test).await;
        let gate = app.api::<dyn KycGate>().unwrap();
        let alice = Subject::User(Uuid::from_u128(2));
        // Quy tắc `kyc.required_activities` (Luật TMĐT 122/2025, từ 01/7/2026): bán hàng cần định danh.
        let error =
            gate.require_verified(DEFAULT_TENANT, alice.clone(), Activity::Sell).await.unwrap_err();
        assert_eq!(error.code(), "compliance.kyc_required");

        let (_, reference) = start(&app, "alice").await;
        // Chữ ký sai: từ chối, không lưu gì.
        let bad = app
            .post("/api/v1/compliance/kyc/callbacks/fake")
            .header("x-fake-signature", "nope")
            .header("content-type", "application/json")
            .bytes(json!({"reference": reference}).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

        verifier.complete(&reference, true, "NGUYEN VAN A", "001099012345");
        assert_eq!(callback(&app, &reference).await, StatusCode::NO_CONTENT);
        // Gọi lại callback: vô hại.
        assert_eq!(callback(&app, &reference).await, StatusCode::NO_CONTENT);
        run_jobs(&app).await;

        let (status, mine) = call(&app, "GET", "/me/kyc", "alice", None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["data"]["status"], "verified");
        let profile = &mine["data"]["profile"];
        assert_eq!(profile["verified_name"], "NGUYEN VAN A");
        // Chỉ lưu 4 số cuối, không lưu số đầy đủ.
        assert!(!mine.to_string().contains("001099012345"), "{mine}");
        assert!(profile["expires_at"].is_string());
        gate.require_verified(DEFAULT_TENANT, alice, Activity::Sell).await.unwrap();

        // Bob dùng cùng giấy tờ: không tự xác thực, chờ duyệt tay.
        let (bob_profile, bob_ref) = start(&app, "bob").await;
        verifier.complete(&bob_ref, true, "NGUYEN VAN A", "001099012345");
        assert_eq!(callback(&app, &bob_ref).await, StatusCode::NO_CONTENT);
        run_jobs(&app).await;
        let (_, bob) =
            call(&app, "GET", &format!("/kyc/profiles/{bob_profile}"), "admin", None).await;
        assert_eq!(bob["data"]["status"], "pending", "{bob}");
        assert_eq!(bob["data"]["reason_code"], "duplicate_document");

        // Duyệt cần người đăng nhập, API key không được.
        let (status, denied) =
            call(&app, "POST", &format!("/kyc/profiles/{bob_profile}/approve"), "robot", None)
                .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(denied["code"], "compliance.session_required");
        let (status, _) =
            call(&app, "POST", &format!("/kyc/profiles/{bob_profile}/approve"), "alice", None)
                .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let body = json!({"reason_code": "duplicate_document"});
        let (status, rejected) =
            call(&app, "POST", &format!("/kyc/profiles/{bob_profile}/reject"), "admin", Some(body))
                .await;
        assert_eq!(status, StatusCode::OK, "{rejected}");
        assert_eq!(rejected["data"]["status"], "rejected");

        let (status, page) =
            call(&app, "GET", "/kyc/profiles?status=verified", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        assert_eq!(page["data"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quy_tac_theo_ngay_hieu_luc_va_nhap_chi_them() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let rules = app.api::<dyn ComplianceRules>().unwrap();
        let before = Utc.with_ymd_and_hms(2026, 6, 30, 16, 59, 0).unwrap();
        let after = Utc.with_ymd_and_hms(2026, 6, 30, 17, 0, 0).unwrap();
        assert!(
            rules
                .effective(DEFAULT_TENANT, "kyc.required_activities", before)
                .await
                .unwrap()
                .is_none()
        );
        let rule = rules
            .effective(DEFAULT_TENANT, "kyc.required_activities", after)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rule.body["sell"], true);
        assert!(rule.legal_basis.contains("122/2025"));
        assert!(rule.needs_legal_review);

        // Nhập cần `compliance.rules.manage` (nhân sự nền tảng); bản đã có giữ nguyên.
        let pack = json!({"pack": {"jurisdiction": "vn", "rules": [{
            "key": "kyc.required_activities", "version": 2,
            "effective_from": "2030-01-01T00:00:00Z", "effective_to": null,
            "body": {"sell": false}, "legal_basis": "Ví dụ — cần đối chiếu văn bản gốc."
        }]}});
        let (status, _) = call(&app, "POST", "/rules", "admin", Some(pack.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, imported) = call(&app, "POST", "/rules", "staff", Some(pack.clone())).await;
        assert_eq!(status, StatusCode::OK, "{imported}");
        assert_eq!(imported["data"]["imported"], 1);
        let (_, again) = call(&app, "POST", "/rules", "staff", Some(pack)).await;
        assert_eq!(again["data"]["imported"], 0);
        let later = Utc.with_ymd_and_hms(2030, 1, 2, 0, 0, 0).unwrap();
        let rule = rules
            .effective(DEFAULT_TENANT, "kyc.required_activities", later)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rule.version, 2);
        // Từ 2030 bán hàng không cần định danh nữa (theo gói vừa nhập).
        assert_eq!(rule.body["sell"], false);
        let (status, listed) = call(&app, "GET", "/rules", "staff", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(listed["data"].as_array().unwrap().iter().any(|rule| rule["version"] == 2));

        let bad = json!({"pack": {"jurisdiction": "vn", "rules": [{
            "key": "x", "version": 0, "effective_from": "2030-01-01T00:00:00Z",
            "body": {}, "legal_basis": ""
        }]}});
        let (status, problem) = call(&app, "POST", "/rules", "staff", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn van_ban_co_phien_ban_va_phai_chap_nhan_lai() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let gate = app.api::<dyn AcceptanceGate>().unwrap();
        let alice = Subject::User(Uuid::from_u128(2));
        // Chưa có văn bản: không chặn.
        gate.require_accepted(DEFAULT_TENANT, alice.clone(), "terms_of_use").await.unwrap();

        let body = json!({"kind": "terms_of_use", "title": "Điều khoản sử dụng"});
        let (status, doc) = call(&app, "POST", "/documents", "admin", Some(body.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{doc}");
        let id = doc["data"]["id"].as_str().unwrap().to_owned();
        let (status, again) = call(&app, "POST", "/documents", "admin", Some(body)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(again["code"], "compliance.document_exists");
        let (status, _) = call(&app, "POST", "/documents", "alice", Some(json!({"kind": "privacy_policy", "title": "x"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let v1 = json!({"locale": "vi", "title": "Điều khoản", "body": "Bản 1"});
        let (status, first) = call(&app, "POST", &format!("/documents/{id}/versions"), "admin", Some(v1)).await;
        assert_eq!(status, StatusCode::CREATED, "{first}");
        assert_eq!(first["data"]["version"], 1);
        assert!(!first["data"]["sha256"].as_str().unwrap().is_empty());
        // Bản dịch cùng số phiên bản; cùng ngôn ngữ lần nữa thì bị từ chối.
        let en = json!({"locale": "en", "title": "Terms", "body": "Version 1", "version": 1});
        let (status, _) = call(&app, "POST", &format!("/documents/{id}/versions"), "admin", Some(en.clone())).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, dup) = call(&app, "POST", &format!("/documents/{id}/versions"), "admin", Some(en)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{dup}");

        let error = gate.require_accepted(DEFAULT_TENANT, alice.clone(), "terms_of_use").await.unwrap_err();
        assert_eq!(error.code(), "compliance.acceptance_required");
        let (_, pending) = call(&app, "GET", "/me/pending-acceptances", "alice", None).await;
        assert_eq!(pending["data"].as_array().unwrap().len(), 1, "{pending}");
        let version_id = pending["data"][0]["version"]["id"].as_str().unwrap().to_owned();
        let (status, accepted) =
            call(&app, "POST", "/me/acceptances", "alice", Some(json!({"version_id": version_id}))).await;
        assert_eq!(status, StatusCode::OK, "{accepted}");
        // Chấp nhận lại: cùng bằng chứng.
        let (_, twice) = call(&app, "POST", "/me/acceptances", "alice", Some(json!({"version_id": version_id}))).await;
        assert_eq!(twice["data"]["id"], accepted["data"]["id"]);
        gate.require_accepted(DEFAULT_TENANT, alice.clone(), "terms_of_use").await.unwrap();

        // Bản 2 không bắt chấp nhận lại: vẫn qua. Bản 3 bắt chấp nhận lại: chặn.
        let v2 = json!({"locale": "vi", "title": "Điều khoản", "body": "Bản 2 (sửa lỗi chính tả)"});
        call(&app, "POST", &format!("/documents/{id}/versions"), "admin", Some(v2)).await;
        gate.require_accepted(DEFAULT_TENANT, alice.clone(), "terms_of_use").await.unwrap();
        let v3 = json!({"locale": "vi", "title": "Điều khoản", "body": "Bản 3", "requires_reacceptance": true});
        call(&app, "POST", &format!("/documents/{id}/versions"), "admin", Some(v3)).await;
        let error = gate.require_accepted(DEFAULT_TENANT, alice.clone(), "terms_of_use").await.unwrap_err();
        assert_eq!(error.code(), "compliance.acceptance_required");
        let accepted = gate.accept_current(DEFAULT_TENANT, alice.clone(), "terms_of_use", "api").await.unwrap();
        assert!(accepted.is_some());
        gate.require_accepted(DEFAULT_TENANT, alice, "terms_of_use").await.unwrap();

        // Trang công khai: bản hiện hành, không cần đăng nhập.
        let public = app.get("/api/v1/compliance/public/documents/terms_of_use?locale=vi").send().await.unwrap();
        assert_eq!(public.status(), StatusCode::OK);
        assert_eq!(public.json()["data"]["version"], 3);

        let (_, stats) = call(&app, "GET", &format!("/documents/{id}/acceptance-stats"), "admin", None).await;
        assert!(stats["data"].as_array().unwrap().iter().any(|row| row["version"] == 3 && row["accepted"] == 1), "{stats}");
        let (_, list) = call(&app, "GET", &format!("/documents/{id}/acceptances"), "admin", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 2, "{list}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_dong_y_rut_o_khoa_rong_hon_thang() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let ledger = app.api::<dyn ConsentLedger>().unwrap();
        let who = ConsentSubject::Email("Khach@Example.com".to_owned());
        let email = ConsentKey::new("marketing".to_owned(), String::new(), "email".to_owned());
        let any = ConsentKey::new("marketing".to_owned(), String::new(), String::new());
        let evidence = ConsentEvidence {
            text: "Tôi đồng ý nhận tin khuyến mãi.".to_owned(),
            text_version: "1".to_owned(),
            source: "newsletter".to_owned(),
            ip_fingerprint: None,
        };
        assert!(!ledger.is_granted(DEFAULT_TENANT, &who, &email).await.unwrap());
        let granted = ledger.grant(DEFAULT_TENANT, who.clone(), any.clone(), evidence.clone(), "form:1").await.unwrap();
        assert_eq!(granted.state, "granted");
        assert_eq!(granted.subject_value.as_deref(), Some("khach@example.com"));
        // Đồng ý ở khoá rộng bao kênh email.
        assert!(ledger.is_granted(DEFAULT_TENANT, &who, &email).await.unwrap());
        // Gọi lại cùng `source_ref`: không ghi thêm.
        let again = ledger.grant(DEFAULT_TENANT, who.clone(), any.clone(), evidence.clone(), "form:1").await.unwrap();
        assert_eq!(again.granted_at, granted.granted_at);
        // Rút ở khoá rộng hơn thắng mọi đồng ý hẹp hơn trước đó.
        ledger.grant(DEFAULT_TENANT, who.clone(), email.clone(), evidence, "form:2").await.unwrap();
        ledger.withdraw(DEFAULT_TENANT, who.clone(), any.clone(), "unsubscribe", "link:1").await.unwrap();
        assert!(!ledger.is_granted(DEFAULT_TENANT, &who, &email).await.unwrap());
        assert!(!ledger.is_granted(DEFAULT_TENANT, &who, &any).await.unwrap());

        // Qua REST: cho đồng ý cần câu đã hiển thị; rút thì không.
        let (status, problem) =
            call(&app, "PUT", "/me/consents", "alice", Some(json!({"purpose": "marketing", "granted": true}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        let body = json!({"purpose": "marketing", "channel": "email", "granted": true, "text": "Đồng ý", "text_version": "1"});
        let (status, mine) = call(&app, "PUT", "/me/consents", "alice", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["data"]["state"], "granted");
        let body = json!({"purpose": "marketing", "channel": "email", "granted": false});
        let (_, mine) = call(&app, "PUT", "/me/consents", "alice", Some(body)).await;
        assert_eq!(mine["data"]["state"], "withdrawn");
        let (_, list) = call(&app, "GET", "/me/consents", "alice", None).await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1, "{list}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn chinh_sach_cua_khach() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let (status, set) = call(
            &app,
            "PUT",
            "/policies/posting.comments",
            "admin",
            Some(json!({"value": "phone"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{set}");
        let (status, _) =
            call(&app, "PUT", "/policies/posting.comments", "admin", Some(json!({"value": "x"})))
                .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, unknown) =
            call(&app, "PUT", "/policies/nope", "admin", Some(json!({"value": 1}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(unknown["code"], "compliance.policy_unknown");
        let (_, list) = call(&app, "GET", "/policies", "admin", None).await;
        assert_eq!(list["data"][0]["value"], "phone");
        let (status, _) = call(&app, "GET", "/policies", "alice", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    })
    .await
    .unwrap();
}
