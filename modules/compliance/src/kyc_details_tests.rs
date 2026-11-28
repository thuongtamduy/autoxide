//! Định danh người bán theo NĐ 248/2026 Điều 18 (LG-3c) và cổng KYC không mở khi quy tắc hỏng (cả SQLite và
//! Postgres).

use autoxide_kernel::identity::{OutcomeStatus, VerificationOutcome};
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_kernel::time::{Date, SignedDuration, Timestamp};
use autoxide_testkit::TestDatabase;
use axum::http::StatusCode;
use sea_orm::{EntityTrait, Set};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Activity, KycGate, Subject};
use crate::tests::{app, call, callback, run_jobs, start};

fn params(error: &autoxide_kernel::error::Error) -> serde_json::Value {
    error.params().map(|(name, value)| (name.to_owned(), json!(value))).collect()
}

/// Thân quy tắc `kyc.required_activities` không đọc được (gói nhập hỏng): cổng báo lỗi, không coi là "không cần
/// định danh" (trước đây fail-open).
#[tokio::test]
async fn quy_tac_kyc_hong_thi_cong_bao_loi() {
    TestDatabase::run(async |test| {
        let (app, _) = app(test).await;
        let at = Timestamp::now();
        crate::entity::rule::Entity::insert(crate::entity::rule::ActiveModel {
            id: Set(Uuid::now_v7()),
            jurisdiction: Set("vn".to_owned()),
            key: Set("kyc.required_activities".to_owned()),
            version: Set(90),
            effective_from: Set(at - SignedDuration::from_hours(1)),
            effective_to: Set(None),
            body: Set("{not json".to_owned()),
            legal_basis: Set("Thử quy tắc hỏng.".to_owned()),
            needs_legal_review: Set(true),
            source: Set("import".to_owned()),
            imported_at: Set(at),
        })
        .exec_without_returning(app.database().write())
        .await
        .unwrap();
        let gate = app.api::<dyn KycGate>().unwrap();
        let alice = Subject::User(Uuid::from_u128(2));
        let error = gate.require_verified(DEFAULT_TENANT, alice, Activity::Sell).await.unwrap_err();
        assert_eq!(error.code(), "internal_error", "{error:?}");
    })
    .await
    .unwrap();
}

/// Quy tắc đòi mục Điều 18 (nhập với ngày hiệu lực đã qua để thử trước 01/01/2027): người đã định danh bằng hộ
/// chiếu nhưng thiếu nước cấp và hạn không bán được (`incomplete`, nêu mục thiếu), việc khác vẫn được; người duyệt
/// ghi mục đã đối chiếu thì bán được, hạn hồ sơ rút theo hạn hộ chiếu. Hạn hộ chiếu dưới 6 tháng tính từ ngày xác
/// thực vẫn bị chặn. API key không ghi được; đầu vào sai báo một lần.
#[tokio::test]
async fn ban_hang_can_du_muc_dieu_18() {
    TestDatabase::run(async |test| {
        let (app, verifier) = app(test).await;
        let pack = json!({"pack": {"jurisdiction": "vn", "rules": [{
            "key": "kyc.required_activities", "version": 9,
            "effective_from": "2026-11-01T00:00:00+07:00", "effective_to": null,
            "body": {"sell": true, "livestream": true, "affiliate": true, "valid_days": 730,
                     "seller_details": {"individual": ["birth_date"],
                                        "organization": ["address", "representative_name"]},
                     "passport_min_valid_months": 6},
            "legal_basis": "Thử — NĐ 248/2026 Điều 18, cần đối chiếu văn bản gốc."
        }]}});
        let (status, imported) = call(&app, "POST", "/rules", "staff", Some(pack)).await;
        assert_eq!(status, StatusCode::OK, "{imported}");

        let (profile, reference) = start(&app, "alice").await;
        let outcome = VerificationOutcome::new(&reference, OutcomeStatus::Verified)
            .verified_name("JANE DOE")
            .document("passport", "C1234567")
            .birth_date(Date::new(1990, 5, 1).unwrap());
        verifier.complete_with(outcome);
        assert_eq!(callback(&app, &reference).await, StatusCode::NO_CONTENT);
        run_jobs(&app).await;

        let gate = app.api::<dyn KycGate>().unwrap();
        let alice = || Subject::User(Uuid::from_u128(2));
        let refused =
            gate.require_verified(DEFAULT_TENANT, alice(), Activity::Sell).await.unwrap_err();
        assert_eq!(refused.code(), "compliance.kyc_required");
        let problem = params(&refused);
        assert_eq!(problem["profile_status"], "incomplete", "{problem}");
        assert_eq!(problem["missing"], "document_country, document_expires_on", "{problem}");
        // Định danh vẫn là định danh: việc không phải bán hàng không bị chặn.
        gate.require_verified(DEFAULT_TENANT, alice(), Activity::Livestream).await.unwrap();

        let uri = format!("/kyc/profiles/{profile}/details");
        let soon = (Timestamp::now() + SignedDuration::from_hours(24 * 30)).date_utc();
        let body = json!({"document_country": "us", "document_expires_on": soon.to_string()});
        let (status, denied) = call(&app, "PUT", &uri, "robot", Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{denied}");
        assert_eq!(denied["code"], "compliance.session_required");
        let bad = json!({"birth_date": "01/05/1990", "document_country": "USA"});
        let (status, problem) = call(&app, "PUT", &uri, "admin", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        assert_eq!(problem["errors"].as_array().unwrap().len(), 2, "{problem}");

        // Hộ chiếu còn chưa tới 6 tháng: vẫn chặn.
        let (status, short) = call(&app, "PUT", &uri, "admin", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{short}");
        assert_eq!(short["data"]["document_country"], "US");
        assert_eq!(short["data"]["birth_date"], "1990-05-01");
        let refused =
            gate.require_verified(DEFAULT_TENANT, alice(), Activity::Sell).await.unwrap_err();
        let problem = params(&refused);
        assert_eq!(problem["missing"], "document_expires_on", "{problem}");
        // Hồ sơ không sống lâu hơn hộ chiếu.
        let expires: Timestamp = short["data"]["expires_at"].as_str().unwrap().parse().unwrap();
        assert_eq!(expires, soon.succ().unwrap().start_of_day_utc());

        let until = (Timestamp::now() + SignedDuration::from_hours(24 * 400)).date_utc();
        let body = json!({"document_expires_on": until.to_string()});
        let (status, done) = call(&app, "PUT", &uri, "admin", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        gate.require_verified(DEFAULT_TENANT, alice(), Activity::Sell).await.unwrap();
        // Mỗi lần ghi mục là một event audit, chỉ có tên mục, không có giá trị.
        use sea_orm::ConnectionTrait as _;
        let read = app.database().read();
        let sql =
            "SELECT payload FROM compliance_events WHERE name = 'compliance.kyc_details_recorded'";
        let statement = sea_orm::Statement::from_string(read.get_database_backend(), sql);
        let rows = read.query_all_raw(statement).await.unwrap();
        assert_eq!(rows.len(), 2);
        let payload: String = rows[0].try_get("", "payload").unwrap();
        assert!(payload.contains("document_country") && !payload.contains("\"US\""), "{payload}");
    })
    .await
    .unwrap();
}
