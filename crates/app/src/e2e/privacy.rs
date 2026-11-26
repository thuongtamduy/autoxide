//! Dữ liệu cá nhân (bước 37, D-360, D-361) trên chương trình thật, cả SQLite và Postgres:
//!
//! - xoá tài khoản (`auth`) thì `compliance` xoá số điện thoại đã xác thực; bản xuất dữ liệu cá nhân có
//!   phần của `compliance` (phần còn lại từ D-356);
//! - yêu cầu `access` của chủ thể qua kênh khác trả bản xuất của mọi module (`auth`, `compliance`…);
//! - khách buộc lưu dữ liệu tại Việt Nam: dời sang placement ở nước khác bị chặn
//!   (`tenant_residency_violation`);
//! - sự cố quá mốc nhắc gửi mail tới chính sách `incident.alert_emails` qua module `mail` thật.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{TenantContext, TenantMoves};
use autoxide_kernel::time::Timestamp;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use jiff::SignedDuration;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

const STAFF: [&str; 5] = [
    "auth.users.manage",
    "compliance.privacy.read",
    "compliance.privacy.manage",
    "compliance.incidents.manage",
    "compliance.policies.manage",
];

async fn app(test: &TestDatabase, dir: &std::path::Path) -> TestApp {
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .tenant("acme")
        .placement("abroad")
        .user("staff", 1, &STAFF);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1{uri}")).token(who);
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

async fn settle(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..8 {
        let jobs = worker.run_once().await.unwrap();
        let mut events = 0;
        while let Ok(sent) = dispatcher.run_once().await {
            if sent == 0 {
                break;
            }
            events += sent;
        }
        if jobs == 0 && events == 0 {
            break;
        }
    }
}

async fn count(tenant: &TenantContext, sql: &str) -> i64 {
    let connection = tenant.database().read();
    let backend = connection.get_database_backend();
    let row =
        connection.query_one_raw(Statement::from_string(backend, sql)).await.unwrap().unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

#[tokio::test]
async fn xoa_tai_khoan_xoa_so_dien_thoai_va_ban_xuat_co_phan_compliance() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-privacy-{}", Uuid::now_v7()));
        let app = app(test, &dir).await;
        let platform = app.default_tenant();
        let account = NewAccount {
            email: "leaving@example.com".to_owned(),
            name: "Leaving".to_owned(),
            password: None,
            admin: false,
        };
        let user = app.api::<dyn Accounts>().unwrap().create(&platform, account).await.unwrap().id;
        // Số đã xác thực (đường mã SMS có bài riêng ở `posting_verification`).
        let write = platform.database().write();
        let backend = write.get_database_backend();
        let now = Timestamp::now();
        let insert = sea_orm::sea_query::Query::insert()
            .into_table(sea_orm::sea_query::Alias::new("compliance_account_verifications"))
            .columns(
                ["tenant_id", "user_id", "phone", "phone_key", "phone_verified_at", "updated_at"]
                    .map(sea_orm::sea_query::Alias::new),
            )
            .values_panic([
                platform.id().as_uuid().into(),
                user.into(),
                "+84912345678".into(),
                "k".into(),
                now.into(),
                now.into(),
            ])
            .to_owned();
        write.execute(&insert).await.unwrap();

        let data = app.modules().personal_data().export(&platform, user).await.unwrap();
        assert_eq!(data["compliance"]["phone_verification"]["phone"], "+84912345678", "{data:?}");

        // Yêu cầu xem dữ liệu qua thư: bản xuất của mọi module.
        let input = json!({"kind": "access", "channel": "post", "subject": user.to_string()});
        let (status, recorded) =
            call(&app, Method::POST, "/compliance/subject-requests", "staff", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED, "{recorded}");
        let id = recorded["data"]["id"].as_str().unwrap();
        let uri = format!("/compliance/subject-requests/{id}/fulfil");
        let (status, done) = call(&app, Method::POST, &uri, "staff", Some(json!({}))).await;
        assert_eq!(status, StatusCode::OK, "{done}");
        assert!(done["data"]["export"]["auth"].is_object(), "{done}");
        assert!(done["data"]["export"]["compliance"]["phone_verification"].is_object(), "{done}");

        let (status, body) =
            call(&app, Method::DELETE, &format!("/auth/users/{user}"), "staff", None).await;
        assert!(status.is_success(), "{status} {body}");
        settle(&app).await;
        let read = platform.database().read();
        let sql = if backend == sea_orm::DbBackend::Postgres {
            "SELECT COUNT(*) AS n FROM compliance_account_verifications WHERE user_id = $1"
        } else {
            "SELECT COUNT(*) AS n FROM compliance_account_verifications WHERE user_id = ?"
        };
        let row = read
            .query_one_raw(autoxide_kernel::database::statement(backend, sql, [user.into()]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<i64>("", "n").unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn khach_luu_tai_viet_nam_khong_doi_sang_placement_nuoc_khac() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-residency-{}", Uuid::now_v7()));
        let app = app(test, &dir).await;
        let acme = app.tenant("acme").await.unwrap();
        let placements = app.modules().placements(app.registry()).unwrap();
        placements.set_country("main", Some("VN")).await.unwrap();
        placements.set_country("abroad", Some("SG")).await.unwrap();
        let body = json!({"tenant_id": acme.id().as_uuid(), "data_residency": "VN"});
        let (status, set) =
            call(&app, Method::PUT, "/compliance/residency", "staff", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{set}");

        let moves = app.api::<TenantMoves>().unwrap();
        let error = moves.start(Actor::system(), "acme", "abroad", false, false).await.unwrap_err();
        assert_eq!(error.code(), "tenant_residency_violation");
        // Bỏ yêu cầu thì dời được (lượt dời bắt đầu).
        let body = json!({"tenant_id": acme.id().as_uuid(), "data_residency": null});
        let (status, _) =
            call(&app, Method::PUT, "/compliance/residency", "staff", Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        let plan =
            moves.plan("acme", "abroad", false, autoxide_kernel::i18n::Language::En).await.unwrap();
        assert!(plan.blocking.iter().all(|blocker| blocker.code != "tenant_residency_violation"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn su_co_qua_moc_gui_mail_toi_nguoi_nhan_canh_bao() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-incident-{}", Uuid::now_v7()));
        let app = app(test, &dir).await;
        let platform = app.default_tenant();
        let policy = json!({"value": ["dpo@example.com"]});
        let (status, body) = call(
            &app,
            Method::PUT,
            "/compliance/policies/incident.alert_emails",
            "staff",
            Some(policy),
        )
        .await;
        assert!(status.is_success(), "{status} {body}");
        let input = json!({
            "title": "Database copy exposed", "severity": "high",
            "detected_at": (Timestamp::now() - SignedDuration::from_hours(25)).to_rfc3339(),
        });
        let (status, opened) = call(&app, Method::POST, "/compliance/incidents", "staff", Some(input)).await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        settle(&app).await;
        let sql = "SELECT COUNT(*) AS n FROM compliance_events WHERE name = 'compliance.incident_deadline_near'";
        assert_eq!(count(&platform, sql).await, 1);
        let mails: Vec<String> = std::fs::read_dir(dir.join("mail"))
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| std::fs::read_to_string(entry.path()).unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            mails.iter().any(|mail| mail.contains("dpo@example.com") && mail.contains("24")),
            "{mails:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    })
    .await
    .unwrap();
}

/// Sổ thời hạn lưu trên chương trình thật có cả `audit`, `webhooks`, `newsletter` (phần còn lại của
/// D-360), và không mục nào vượt giới hạn mặc định của quy tắc.
#[tokio::test]
async fn so_thoi_han_luu_co_audit_webhooks_newsletter() {
    TestDatabase::run(async |test| {
        let dir = std::env::temp_dir().join(format!("autoxide-privacy-{}", Uuid::now_v7()));
        let app = app(test, &dir).await;
        let (status, body) =
            call(&app, Method::GET, "/compliance/retention?limit=200", "staff", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let items = body["data"].as_array().unwrap();
        let find = |name: &str| {
            items.iter().find(|item| item["name"] == name).unwrap_or_else(|| panic!("{name} thiếu"))
        };
        assert_eq!(find("audit.entries")["keep"], "until_deleted");
        assert_eq!(find("webhooks.deliveries")["days"], 30);
        assert_eq!(find("newsletter.subscribers")["keep"], "until_deleted");
        let unconfirmed = find("newsletter.unconfirmed");
        assert_eq!(unconfirmed["days"], 30);
        assert_eq!(unconfirmed["setting"], "AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS");
        for name in [
            "audit.entries",
            "webhooks.deliveries",
            "newsletter.subscribers",
            "newsletter.unconfirmed",
        ] {
            assert_eq!(find(name)["status"], "ok", "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    })
    .await
    .unwrap();
}
