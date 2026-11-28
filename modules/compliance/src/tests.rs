//! Test của module `compliance` qua REST và trait công bố (cả SQLite và Postgres): định danh qua nhà
//! cung cấp giả (callback có chữ ký → job → `verified`), trùng giấy tờ phải duyệt tay, duyệt chỉ bởi
//! người đăng nhập, cổng KYC theo quy tắc có ngày hiệu lực, văn bản có phiên bản + chấp nhận lại, sổ
//! đồng ý (rút ở khoá rộng hơn thắng), nhập quy tắc chỉ thêm.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::identity::IdentityVerifier;
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_testkit::{FakeIdentityVerifier, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
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

pub(crate) async fn app(test: &TestDatabase) -> (TestApp, Arc<FakeIdentityVerifier>) {
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
async fn nhan_vien_dinh_danh_to_chuc_qua_dang_ky_kinh_doanh() {
    TestDatabase::run(async |test| {
        let (app, verifier) = app(test).await;
        // Hồ sơ của người bán là tổ chức (nút gốc), nhân viên có `compliance.kyc.submit` mở và nộp.
        let body = json!({"party_type": "organization"});
        let (status, opened) = call(&app, "POST", "/kyc/profiles", "admin", Some(body)).await;
        assert!(status.is_success(), "{opened}");
        let profile = opened["data"]["id"].as_str().unwrap().to_owned();
        let body = json!({"method": "business_registry", "return_url": "https://shop.example/kyc"});
        let uri = format!("/kyc/profiles/{profile}/sessions");
        let (status, session) = call(&app, "POST", &uri, "admin", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{session}");
        assert_eq!(session["data"]["profile_id"], profile.as_str());
        let attempt: Uuid = session["data"]["attempt_id"].as_str().unwrap().parse().unwrap();
        // Người không có quyền nộp: 403.
        let (status, _) = call(&app, "POST", &uri, "alice", Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let reference = FakeIdentityVerifier::reference(attempt);
        verifier.complete(&reference, true, "CONG TY TNHH AN", "0101234567");
        assert_eq!(callback(&app, &reference).await, StatusCode::NO_CONTENT);
        run_jobs(&app).await;
        let (_, read) = call(&app, "GET", &format!("/kyc/profiles/{profile}"), "admin", None).await;
        assert_eq!(read["data"]["verified_name"], "CONG TY TNHH AN", "{read}");
    })
    .await
    .unwrap();
}

/// Rà pháp lý vòng 1 (D-20261126-legal-rules-r1-update): căn cứ mới là phiên bản quy tắc mới có ngày hiệu lực,
/// phiên bản cũ vẫn áp cho thời điểm trước — ranh giới 00:00 ngày 01/7/2026 giờ Việt Nam.
#[tokio::test]
async fn quy_tac_ra_phap_ly_vong_1_theo_ngay_hieu_luc() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let rules = app.api::<dyn ComplianceRules>().unwrap();
        let before = Timestamp::from_utc(2026, 6, 30, 16, 59, 0).unwrap();
        let after = Timestamp::from_utc(2026, 6, 30, 17, 0, 0).unwrap();
        for key in ["moit.checklist.sales", "moit.checklist.platform", "cases.deadlines"] {
            let old = rules.effective(DEFAULT_TENANT, key, before).await.unwrap().unwrap();
            assert_eq!(old.version, 1, "{key}");
            let new = rules.effective(DEFAULT_TENANT, key, after).await.unwrap().unwrap();
            assert_eq!(new.version, 2, "{key}");
            assert!(new.legal_basis.contains("248/2026"), "{key}: {}", new.legal_basis);
            assert!(new.needs_legal_review, "{key}");
        }
        let checklist =
            rules.effective(DEFAULT_TENANT, "moit.checklist.sales", after).await.unwrap().unwrap();
        let items = checklist.body["items"].as_array().unwrap();
        assert!(
            items
                .iter()
                .all(|item| !item["legal_basis"].as_str().unwrap().contains("52/2013/NĐ-CP về"))
        );
        let requests = rules
            .effective(DEFAULT_TENANT, "privacy.subject_requests", after)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(requests.version, 3);
        assert!(requests.legal_basis.contains("356/2025"));
        // Hạn trả lời giữ 72 giờ (chặt hơn nghị định); hạn thực hiện tính theo ngày thường (LG-4b).
        assert_eq!(requests.body["respond_within_hours"]["erasure"], 72);
        assert_eq!(requests.body["fulfil_within_days"]["erasure"], 20);
        assert_eq!(requests.body["fulfil_unit"], "days");
        // Định danh người bán: mục NĐ 248/2026 Điều 18 từ 00:00 ngày 01/01/2027 giờ Việt Nam (LG-3c).
        let kyc = |at| rules.effective(DEFAULT_TENANT, "kyc.required_activities", at);
        let end_2026 = Timestamp::from_utc(2026, 12, 31, 16, 59, 0).unwrap();
        let new_year = Timestamp::from_utc(2026, 12, 31, 17, 0, 0).unwrap();
        assert_eq!(kyc(end_2026).await.unwrap().unwrap().version, 1);
        let v2 = kyc(new_year).await.unwrap().unwrap();
        assert_eq!(v2.version, 2);
        assert!(v2.legal_basis.contains("248/2026"));
        assert_eq!(v2.body["seller_details"]["individual"], json!(["birth_date"]));
        assert_eq!(v2.body["passport_min_valid_months"], 6);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quy_tac_theo_ngay_hieu_luc_va_nhap_chi_them() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let rules = app.api::<dyn ComplianceRules>().unwrap();
        let before = Timestamp::from_utc(2026, 6, 30, 16, 59, 0).unwrap();
        let after = Timestamp::from_utc(2026, 6, 30, 17, 0, 0).unwrap();
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
            "key": "kyc.required_activities", "version": 3,
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
        let later = Timestamp::from_utc(2030, 1, 2, 0, 0, 0).unwrap();
        let rule = rules
            .effective(DEFAULT_TENANT, "kyc.required_activities", later)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rule.version, 3);
        // Từ 2030 bán hàng không cần định danh nữa (theo gói vừa nhập).
        assert_eq!(rule.body["sell"], false);
        let (status, listed) = call(&app, "GET", "/rules", "staff", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert!(listed["data"].as_array().unwrap().iter().any(|rule| rule["version"] == 3));
        // `as_of`: mỗi khoá chỉ còn bản có hiệu lực lúc đó.
        let uri = "/rules?key=kyc.required_activities&jurisdiction=vn&as_of=2030-01-02T00:00:00Z";
        let (status, at) = call(&app, "GET", uri, "staff", None).await;
        assert_eq!(status, StatusCode::OK, "{at}");
        let versions: Vec<i64> =
            at["data"].as_array().unwrap().iter().map(|r| r["version"].as_i64().unwrap()).collect();
        assert_eq!(versions, [3]);
        let uri = "/rules?key=kyc.required_activities&as_of=2029-12-31T00:00:00Z";
        let (_, before) = call(&app, "GET", uri, "staff", None).await;
        assert!(before["data"].as_array().unwrap().iter().all(|rule| rule["version"] != 3));
        let (_, all) = call(&app, "GET", "/rules?as_of=2030-01-02T00:00:00Z", "staff", None).await;
        let mut keys: Vec<(String, String)> = all["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["jurisdiction"].to_string(), r["key"].to_string()))
            .collect();
        let count = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), count, "one version per rule: {all}");
        let (status, problem) = call(&app, "GET", "/rules?as_of=yesterday", "staff", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");

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

/// Module `auth` giả cho test: chỉ đăng ký bộ tra tên tài khoản `auth.user`.
struct FakeAuth;

struct FakeNames;

#[async_trait::async_trait]
impl autoxide_kernel::resource::ResourceResolver for FakeNames {
    async fn resolve(
        &self,
        _tenant: &autoxide_kernel::tenancy::TenantContext,
        id: &str,
    ) -> Result<Option<autoxide_kernel::resource::Resource>, autoxide_kernel::error::Error> {
        Ok((id == Uuid::from_u128(2).to_string())
            .then(|| autoxide_kernel::resource::Resource::new("Alice Nguyen")))
    }
}

impl autoxide_kernel::module::Module for FakeAuth {
    fn name(&self) -> &'static str {
        "auth"
    }

    fn start(
        &mut self,
        context: &mut autoxide_kernel::module::StartContext<'_>,
    ) -> Result<(), autoxide_kernel::error::Error> {
        context.resource_type("auth.user", Arc::new(FakeNames))
    }
}

#[tokio::test]
async fn ho_so_kyc_kem_ten_chu_the_co_thieu_giay_to_va_ghi_chu_duyet() {
    TestDatabase::run(async |test| {
        let verifier = Arc::new(FakeIdentityVerifier::new());
        let app = TestApp::builder(test)
            .user("admin", 1, ALL)
            .user("alice", 2, &[])
            .provide::<dyn IdentityVerifier>(verifier)
            .module(FakeAuth)
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let (status, opened) = call(&app, "POST", "/me/kyc", "alice", Some(json!({}))).await;
        assert!(status.is_success(), "{opened}");

        let (status, page) = call(&app, "GET", "/kyc/profiles?status=draft", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{page}");
        let profile = &page["data"][0];
        assert_eq!(profile["subject_kind"], "user");
        assert_eq!(profile["subject_name"], "Alice Nguyen");
        assert_eq!(profile["missing_documents"], true);
        assert_eq!(profile["review_note"], Value::Null);
        let id = profile["id"].as_str().unwrap().to_owned();
        let (_, one) = call(&app, "GET", &format!("/kyc/profiles/{id}"), "admin", None).await;
        assert_eq!(one["data"]["subject_name"], "Alice Nguyen");

        // Sai cả mã lẫn ghi chú: báo một lần.
        let long = "x".repeat(1001);
        let body = json!({"reason_code": "", "note": long});
        let uri = format!("/kyc/profiles/{id}/reject");
        let (status, problem) = call(&app, "POST", &uri, "admin", Some(body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2);

        let body = json!({"reason_code": "document_unreadable",
                          "note": "The photo of the ID card is blurred; send a sharper one."});
        let (status, rejected) = call(&app, "POST", &uri, "admin", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{rejected}");
        assert_eq!(rejected["data"]["status"], "rejected");
        assert_eq!(rejected["data"]["missing_documents"], false);
        assert_eq!(
            rejected["data"]["review_note"],
            "The photo of the ID card is blurred; send a sharper one."
        );
        // Người được duyệt đọc được ghi chú.
        let (_, mine) = call(&app, "GET", "/me/kyc", "alice", None).await;
        assert_eq!(mine["data"]["profile"]["review_note"], rejected["data"]["review_note"]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_callback_cung_luot_chi_phat_mot_event() {
    use sea_orm::{ConnectionTrait, Statement};
    TestDatabase::run(async |test| {
        let (app, verifier) = app(test).await;
        let (_, reference) = start(&app, "alice").await;
        verifier.complete(&reference, true, "NGUYEN VAN A", "001099012345");
        // Nhà cung cấp gọi callback hai lần (gửi lại khi chưa nhận được trả lời), hai job xử lý chạy
        // cùng lúc. Postgres từng cho cả hai cùng thấy lượt `pending` và cùng phát
        // `compliance.kyc_verified`.
        let (first, second) = tokio::join!(callback(&app, &reference), callback(&app, &reference));
        assert_eq!((first, second), (StatusCode::NO_CONTENT, StatusCode::NO_CONTENT));
        // Mỗi callback một job; worker chạy hai job cùng lúc.
        run_jobs(&app).await;
        let read = app.database().read();
        let sql = "SELECT name FROM compliance_events WHERE name = 'compliance.kyc_verified'";
        let rows = read
            .query_all_raw(Statement::from_string(read.get_database_backend(), sql))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn ket_qua_muon_cua_luot_cu_khong_xac_thuc_lai_ho_so_da_thu_hoi() {
    TestDatabase::run(async |test| {
        let (app, verifier) = app(test).await;
        let (profile, reference) = start(&app, "alice").await;
        // Người duyệt xác thực tay khi lượt ở nhà cung cấp còn chờ, rồi thu hồi.
        let (status, approved) =
            call(&app, "POST", &format!("/kyc/profiles/{profile}/approve"), "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        let body = json!({"reason_code": "forged_document"});
        let (status, revoked) =
            call(&app, "POST", &format!("/kyc/profiles/{profile}/revoke"), "admin", Some(body))
                .await;
        assert_eq!(status, StatusCode::OK, "{revoked}");
        // Kết quả của lượt cũ về muộn: trước đây xác thực lại hồ sơ vừa bị thu hồi (cả hai DB).
        verifier.complete(&reference, true, "NGUYEN VAN A", "001099012345");
        assert_eq!(callback(&app, &reference).await, StatusCode::NO_CONTENT);
        run_jobs(&app).await;
        let (_, found) =
            call(&app, "GET", &format!("/kyc/profiles/{profile}"), "admin", None).await;
        assert_eq!(found["data"]["status"], "revoked", "{found}");

        // Lượt cũ khi đã mở lượt mới: kết quả của lượt cũ không quyết hồ sơ.
        let (_, old) = start(&app, "bob").await;
        let (bob, _) = start(&app, "bob").await;
        verifier.complete(&old, true, "TRAN VAN B", "001099054321");
        assert_eq!(callback(&app, &old).await, StatusCode::NO_CONTENT);
        run_jobs(&app).await;
        let (_, found) = call(&app, "GET", &format!("/kyc/profiles/{bob}"), "admin", None).await;
        assert_eq!(found["data"]["status"], "pending", "{found}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn hai_ho_so_cung_giay_to_xac_thuc_cung_luc_chi_mot_ho_so_verified() {
    TestDatabase::run(async |test| {
        let (app, verifier) = app(test).await;
        for (pair, document) in
            [(["alice", "bob"], "001099012345"), (["admin", "staff"], "001099067890")]
        {
            let mut references = Vec::new();
            let mut profiles = Vec::new();
            for who in pair {
                let (profile, reference) = start(&app, who).await;
                verifier.complete(&reference, true, "NGUYEN VAN A", document);
                profiles.push(profile);
                references.push(reference);
            }
            // Hai callback, hai job chạy cùng lúc. Trước đây mỗi bên khoá hồ sơ của mình, cùng thấy
            // "chưa ai dùng giấy tờ này" và cùng `verified`.
            let (one, two) =
                tokio::join!(callback(&app, &references[0]), callback(&app, &references[1]));
            assert_eq!((one, two), (StatusCode::NO_CONTENT, StatusCode::NO_CONTENT));
            run_jobs(&app).await;
            let mut statuses = Vec::new();
            for profile in &profiles {
                let (_, found) =
                    call(&app, "GET", &format!("/kyc/profiles/{profile}"), "admin", None).await;
                statuses.push((
                    found["data"]["status"].as_str().unwrap().to_owned(),
                    found["data"]["reason_code"].as_str().map(str::to_owned),
                ));
            }
            statuses.sort();
            assert_eq!(
                statuses,
                vec![
                    ("pending".to_owned(), Some("duplicate_document".to_owned())),
                    ("verified".to_owned(), None),
                ]
            );
        }
    })
    .await
    .unwrap();
}
