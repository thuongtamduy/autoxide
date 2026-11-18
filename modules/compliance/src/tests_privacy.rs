//! Test dữ liệu cá nhân (D-360) trên cả SQLite và Postgres: sổ mục đích (mặc định + khách ghi đè),
//! thời hạn lưu theo sổ của kernel, nơi lưu theo khách (chỉ nhân sự nền tảng đặt, từ chối khi
//! placement không đáp ứng), sự cố có hạn 72 giờ từ lúc phát hiện và nhắc đúng mốc không trùng, bản
//! nháp thông báo, sổ yêu cầu của chủ thể qua kênh khác (hạn là dữ liệu, xoá giá trị trong sổ đồng ý).

use autoxide_kernel::database::Database;
use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Statement};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::api::{ConsentEvidence, ConsentKey, ConsentLedger, ConsentSubject};
use crate::entity::{consent, incident};
use crate::tests::{call, run_jobs};

const PRIVACY: &[&str] = &[
    "compliance.privacy.read",
    "compliance.privacy.manage",
    "compliance.incidents.manage",
    "compliance.policies.manage",
];

async fn app(test: &TestDatabase) -> TestApp {
    TestApp::builder(test)
        .user("admin", 1, PRIVACY)
        .user("alice", 2, &[])
        .module(crate::module())
        .build()
        .await
        .unwrap()
}

async fn events(database: &Database, name: &str) -> usize {
    let backend = database.read().get_database_backend();
    let sql = format!("SELECT name FROM compliance_events WHERE name = '{name}'");
    database.read().query_all_raw(Statement::from_string(backend, sql)).await.unwrap().len()
}

fn at(value: &serde_json::Value) -> DateTime<Utc> {
    value.as_str().unwrap().parse().unwrap()
}

#[tokio::test]
async fn muc_dich_mac_dinh_khach_ghi_de_va_cong_khai() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, _) = call(&app, "GET", "/purposes", "alice", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, listed) = call(&app, "GET", "/purposes", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let keys: Vec<&str> = listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["key"].as_str().unwrap())
            .collect();
        assert!(keys.contains(&"marketing") && keys.contains(&"service"), "{listed}");

        // Sai nhiều chỗ: báo một lần.
        let bad = json!({"key": "Bad", "category": "ads", "legal_basis": "x", "description": {}});
        let (status, problem) = call(&app, "POST", "/purposes", "admin", Some(bad)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(problem["errors"].as_array().unwrap().len(), 4, "{problem}");

        let loyalty = json!({
            "key": "loyalty", "category": "marketing", "legal_basis": "consent",
            "data_categories": ["email", "purchase_history"],
            "recipients": [{"name": "Mail provider", "country": "SG"}],
            "description": {"en": "Loyalty points.", "vi": "Điểm thưởng."},
        });
        let (status, created) =
            call(&app, "POST", "/purposes", "admin", Some(loyalty.clone())).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let (status, _) = call(&app, "POST", "/purposes", "admin", Some(loyalty)).await;
        assert_eq!(status, StatusCode::CONFLICT);

        // Ghi đè mục mặc định, rồi tắt.
        let (status, changed) =
            call(&app, "PATCH", "/purposes/analytics", "admin", Some(json!({"active": false})))
                .await;
        assert_eq!(status, StatusCode::OK, "{changed}");
        assert_eq!(changed["data"]["source"], "tenant");
        let (status, _) = call(&app, "PATCH", "/purposes/nope", "admin", Some(json!({}))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let public = app.get("/api/v1/compliance/public/purposes").send().await.unwrap();
        assert_eq!(public.status(), StatusCode::OK);
        let public = public.json();
        let keys: Vec<&str> = public["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["key"].as_str().unwrap())
            .collect();
        assert!(keys.contains(&"loyalty") && !keys.contains(&"analytics"), "{public}");
        assert_eq!(events(app.database(), "compliance.purpose_updated").await, 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thoi_han_luu_va_so_hoat_dong_xu_ly() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, items) = call(&app, "GET", "/retention", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{items}");
        let items = items["data"].as_array().unwrap();
        let callbacks =
            items.iter().find(|item| item["name"] == "compliance.kyc_callbacks").unwrap();
        assert_eq!(callbacks["keep"], "days");
        assert_eq!(callbacks["days"], 90);
        assert_eq!(callbacks["setting"], "AUTOXIDE_COMPLIANCE_KYC_CALLBACK_RETENTION_DAYS");
        assert_eq!(callbacks["status"], "ok");
        // Bằng chứng (loại `accounting`) được giữ mãi theo quy tắc.
        let proof = items.iter().find(|item| item["name"] == "compliance.consents").unwrap();
        assert_eq!(proof["status"], "ok");
        let cookies =
            items.iter().find(|item| item["name"] == "compliance.cookie_consents").unwrap();
        assert_eq!(cookies["days"], 1825);

        let (status, register) = call(&app, "GET", "/processing-register", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{register}");
        assert!(register["data"]["purposes"].as_array().unwrap().len() >= 4);
        assert_eq!(register["data"]["residency"]["compliant"], true);
        assert!(register["data"]["disclaimer"].as_str().unwrap().contains("legal professional"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn noi_luu_chi_dat_duoc_khi_placement_dap_ung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, found) = call(&app, "GET", "/residency", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{found}");
        assert_eq!(found["data"]["data_residency"], serde_json::Value::Null);
        assert_eq!(found["data"]["placement"], "main");

        let (status, _) =
            call(&app, "PUT", "/residency", "alice", Some(json!({"data_residency": "VN"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, problem) =
            call(&app, "PUT", "/residency", "admin", Some(json!({"data_residency": "VNM"}))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{problem}");
        // Placement chưa ghi nước: không đáp ứng.
        let (status, problem) =
            call(&app, "PUT", "/residency", "admin", Some(json!({"data_residency": "vn"}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");
        assert_eq!(problem["code"], "tenant_residency_violation");

        let placements = app.modules().placements(app.registry()).unwrap();
        placements.set_country("main", Some("VN")).await.unwrap();
        let (status, set) =
            call(&app, "PUT", "/residency", "admin", Some(json!({"data_residency": "vn"}))).await;
        assert_eq!(status, StatusCode::OK, "{set}");
        assert_eq!(set["data"]["data_residency"], "VN");
        assert_eq!(set["data"]["compliant"], true);
        // Đổi nước của placement đang giữ khách buộc lưu tại VN: từ chối.
        let error = placements.set_country("main", Some("SG")).await.unwrap_err();
        assert_eq!(error.code(), "tenant_residency_violation");
        assert_eq!(events(app.database(), "compliance.residency_changed").await, 1);
    })
    .await
    .unwrap();
}

#[derive(Serialize)]
struct Deadlines;

impl Job for Deadlines {
    const KIND: &'static str = "compliance.incident_deadlines";
}

async fn enqueue_deadlines(app: &TestApp) {
    JobQueue::new("compliance")
        .enqueue_database(app.database().write(), &Deadlines, EnqueueOptions::default())
        .await
        .unwrap();
}

#[tokio::test]
async fn han_72_gio_tinh_tu_luc_phat_hien() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let detected = (Utc::now() - Duration::hours(10)).to_rfc3339();
        let input = json!({
            "title": "Lost laptop", "severity": "high", "detected_at": detected,
            "data_categories": ["email", "phone"], "affected_count": 120,
        });
        let (status, opened) = call(&app, "POST", "/incidents", "admin", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        let due = at(&opened["data"]["report_due_at"]) - at(&opened["data"]["detected_at"]);
        assert_eq!(due, Duration::hours(72));
        assert_eq!(opened["data"]["overdue"], false);

        let future = json!({"title": "x", "severity": "low", "detected_at": (Utc::now() + Duration::hours(2)).to_rfc3339()});
        let (status, _) = call(&app, "POST", "/incidents", "admin", Some(future)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

        let id = opened["data"]["id"].as_str().unwrap();
        let (status, _) = call(&app, "GET", &format!("/incidents/{id}"), "alice", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let uri = format!("/incidents/{id}/entries");
        let (status, _) = call(&app, "POST", &uri, "admin", Some(json!({"body": "Disk was encrypted."}))).await;
        assert_eq!(status, StatusCode::CREATED);
        let (status, draft) =
            call(&app, "GET", &format!("/incidents/{id}/draft?audience=authority&locale=vi"), "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{draft}");
        let text = draft["data"]["text"].as_str().unwrap();
        assert!(text.contains("Lost laptop") && text.contains("120") && text.contains("email, phone"), "{text}");
        assert_eq!(draft["data"]["locale"], "vi");

        // Báo cơ quan: chuyển `reported`.
        let report = json!({"authority_notified_at": Utc::now().to_rfc3339(), "authority_reference": "A-1"});
        let (status, reported) = call(&app, "PATCH", &format!("/incidents/{id}"), "admin", Some(report)).await;
        assert_eq!(status, StatusCode::OK, "{reported}");
        assert_eq!(reported["data"]["status"], "reported");
        let (_, detail) = call(&app, "GET", &format!("/incidents/{id}"), "admin", None).await;
        assert_eq!(detail["data"]["entries"].as_array().unwrap().len(), 1);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn nhac_han_dung_moc_khong_nhac_trung() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let input = json!({
            "title": "Leak", "severity": "critical",
            "detected_at": (Utc::now() - Duration::hours(50)).to_rfc3339(),
        });
        let (status, opened) = call(&app, "POST", "/incidents", "admin", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        let id: Uuid = opened["data"]["id"].as_str().unwrap().parse().unwrap();

        run_jobs(&app).await;
        assert_eq!(events(app.database(), "compliance.incident_deadline_near").await, 1);
        let row =
            incident::Entity::find_by_id(id).one(app.database().read()).await.unwrap().unwrap();
        assert_eq!(row.reminded_hours, 48);
        // Chạy lại cùng mốc: không nhắc lần hai.
        enqueue_deadlines(&app).await;
        run_jobs(&app).await;
        assert_eq!(events(app.database(), "compliance.incident_deadline_near").await, 1);
        // Qua mốc 60 giờ: nhắc một lần nữa.
        incident::Entity::update_many()
            .col_expr(incident::Column::DetectedAt, Expr::value(Utc::now() - Duration::hours(61)))
            .filter(incident::Column::Id.eq(id))
            .exec(app.database().write())
            .await
            .unwrap();
        enqueue_deadlines(&app).await;
        run_jobs(&app).await;
        assert_eq!(events(app.database(), "compliance.incident_deadline_near").await, 2);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn so_yeu_cau_chu_the_han_la_du_lieu_va_xoa_gia_tri() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ledger = app.api::<dyn ConsentLedger>().unwrap();
        let email = ConsentSubject::Email("ada@example.com".to_owned());
        let evidence = ConsentEvidence {
            text: "Send me news".to_owned(),
            text_version: "1".to_owned(),
            source: "web_form".to_owned(),
            ip_fingerprint: None,
        };
        ledger
            .grant(
                autoxide_kernel::tenancy::DEFAULT_TENANT,
                email.clone(),
                ConsentKey::new("marketing", "", "email"),
                evidence,
                "test:1",
            )
            .await
            .unwrap();

        let (status, _) = call(&app, "GET", "/subject-requests", "alice", None).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let received = Utc::now() - Duration::hours(2);
        let input = json!({"kind": "erasure", "channel": "email", "subject": " Ada@Example.com ", "received_at": received.to_rfc3339()});
        let (status, recorded) = call(&app, "POST", "/subject-requests", "admin", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED, "{recorded}");
        assert_eq!(recorded["data"]["subject_kind"], "email");
        assert_eq!(recorded["data"]["subject"], "ada@example.com");
        let due = at(&recorded["data"]["due_at"]) - at(&recorded["data"]["received_at"]);
        assert_eq!(due, Duration::hours(72), "{recorded}");

        let id = recorded["data"]["id"].as_str().unwrap();
        let (status, done) =
            call(&app, "POST", &format!("/subject-requests/{id}/fulfil"), "admin", Some(json!({"notes": "Done by mail."}))).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert_eq!(done["data"]["request"]["status"], "fulfilled");
        assert_eq!(done["data"]["erased"], 1);
        let rows = consent::Entity::find().all(app.database().read()).await.unwrap();
        assert!(rows.iter().all(|row| row.subject_value.is_none()), "{rows:?}");
        let (status, problem) =
            call(&app, "POST", &format!("/subject-requests/{id}/fulfil"), "admin", Some(json!({}))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{problem}");

        // Yêu cầu xem dữ liệu của người dùng: bản xuất của mọi module (ở đây chỉ `compliance`).
        let user = Uuid::from_u128(2).to_string();
        let input = json!({"kind": "access", "channel": "post", "subject": user});
        let (_, recorded) = call(&app, "POST", "/subject-requests", "admin", Some(input)).await;
        let id = recorded["data"]["id"].as_str().unwrap();
        let reject = json!({"status": "rejected"});
        let (status, _) = call(&app, "PATCH", &format!("/subject-requests/{id}"), "admin", Some(reject)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, done) =
            call(&app, "POST", &format!("/subject-requests/{id}/fulfil"), "admin", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert!(done["data"]["export"]["compliance"]["consents"].is_array(), "{done}");

        // Quá hạn: lọc được.
        let old = json!({"kind": "objection", "channel": "phone", "subject": "0912345678", "received_at": (Utc::now() - Duration::days(5)).to_rfc3339()});
        let (_, late) = call(&app, "POST", "/subject-requests", "admin", Some(old)).await;
        assert_eq!(late["data"]["overdue"], true, "{late}");
        let (_, overdue) = call(&app, "GET", "/subject-requests?overdue=true", "admin", None).await;
        assert_eq!(overdue["data"].as_array().unwrap().len(), 1, "{overdue}");
        assert_eq!(events(app.database(), "compliance.subject_request_received").await, 3);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn muc_dich_kem_so_nguoi_dong_y_va_da_rut() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ledger = app.api::<dyn ConsentLedger>().unwrap();
        let tenant = autoxide_kernel::tenancy::DEFAULT_TENANT;
        let evidence = || ConsentEvidence {
            text: "Send me news".to_owned(),
            text_version: "1".to_owned(),
            source: "web_form".to_owned(),
            ip_fingerprint: None,
        };
        // Ada đồng ý qua hai kênh (vẫn là một người); Bob đồng ý rồi rút; Chi chỉ đồng ý phân tích.
        let subject = |email: &str| ConsentSubject::Email(email.to_owned());
        for (index, (email, channel)) in
            [("ada@example.com", "email"), ("ada@example.com", "sms"), ("bob@example.com", "email")]
                .into_iter()
                .enumerate()
        {
            ledger
                .grant(
                    tenant,
                    subject(email),
                    ConsentKey::new("marketing", "", channel),
                    evidence(),
                    &format!("test:{index}"),
                )
                .await
                .unwrap();
        }
        ledger
            .withdraw(
                tenant,
                subject("bob@example.com"),
                ConsentKey::new("marketing", "", "email"),
                "web_form",
                "test:withdraw",
            )
            .await
            .unwrap();
        ledger
            .grant(
                tenant,
                subject("chi@example.com"),
                ConsentKey::new("analytics", "", ""),
                evidence(),
                "test:chi",
            )
            .await
            .unwrap();

        let (status, listed) = call(&app, "GET", "/purposes", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let counts = |key: &str| {
            let item = listed["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["key"] == key)
                .unwrap_or_else(|| panic!("no purpose {key}: {listed}"));
            (item["consented"].as_i64().unwrap(), item["withdrawn"].as_i64().unwrap())
        };
        assert_eq!(counts("marketing"), (1, 1));
        assert_eq!(counts("analytics"), (1, 0));
        // Trang công khai không lộ số đếm.
        let public = app.get("/api/v1/compliance/public/purposes").send().await.unwrap();
        let public: serde_json::Value = public.json();
        assert!(public["data"][0].get("consented").is_none(), "{public}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn tong_quan_chi_dem_khu_vuc_duoc_doc() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let received = Utc::now() - Duration::days(5);
        let input = json!({"kind": "access", "channel": "email", "subject": "ada@example.com",
                           "received_at": received.to_rfc3339()});
        let (status, created) = call(&app, "POST", "/subject-requests", "admin", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let input = json!({"kind": "erasure", "channel": "email", "subject": "bob@example.com"});
        let (status, _) = call(&app, "POST", "/subject-requests", "admin", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, overview) = call(&app, "GET", "/overview", "admin", None).await;
        assert_eq!(status, StatusCode::OK, "{overview}");
        let data = &overview["data"];
        assert_eq!(data["subject_requests"], json!({"open": 2, "overdue": 1}));
        assert_eq!(data["incidents"], json!({"open": 0, "report_overdue": 0}));
        // Không có quyền đọc khu vực: `null`.
        assert_eq!(data["kyc"], serde_json::Value::Null);
        assert_eq!(data["cases"], serde_json::Value::Null);
        assert_eq!(data["websites"], serde_json::Value::Null);

        let (status, nothing) = call(&app, "GET", "/overview", "alice", None).await;
        assert_eq!(status, StatusCode::OK, "{nothing}");
        assert_eq!(nothing["data"]["subject_requests"], serde_json::Value::Null);
    })
    .await
    .unwrap();
}
