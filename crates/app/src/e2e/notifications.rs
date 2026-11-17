//! Thông báo (bước 37 phần B, D-348) trên chương trình thật: thông báo của nền tảng tới quản trị viên
//! mọi khách (quản trị viên thật của `auth`), thuê bao bị tạm khoá thì `plans` báo quản trị viên khách,
//! rút đồng ý trong sổ của `compliance` huỷ tin quảng cáo đang chờ, quy tắc khung giờ đi kèm. Chạy
//! trên cả SQLite và Postgres.

use autoxide_auth::api::{Accounts, NewAccount};
use autoxide_compliance::api::{
    ComplianceRules, ConsentEvidence, ConsentKey, ConsentLedger, ConsentSubject,
};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, TenantContext};
use autoxide_notifications::api::{
    Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient,
};
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

const OPS: [&str; 4] = [
    "notifications.platform_notices.send",
    "plans.plans.manage",
    "plans.subscriptions.manage",
    "kernel.tenants.read",
];

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-notifications-{}", Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_PLANS_CACHE_SECONDS", "0")
        .tenant("acme")
        .tenant("beta")
        .user("ops", 1, &OPS)
        .user_in("acme", "owner", 2, &["notifications.platform_notices.send"]);
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

/// Tài khoản thật trong khách `tenant`.
async fn account(app: &TestApp, tenant: &TenantContext, email: &str, admin: bool) -> Uuid {
    let account =
        NewAccount { email: email.to_owned(), name: email.to_owned(), password: None, admin };
    app.api::<dyn Accounts>().unwrap().create(tenant, account).await.unwrap().id
}

/// Chạy job và phát event tới khi hết việc.
async fn settle(app: &TestApp) {
    let worker = app.modules().worker(4).unwrap().blocked_delay(std::time::Duration::ZERO);
    let dispatcher = app.modules().dispatcher().unwrap();
    for _ in 0..8 {
        let jobs = worker.run_once().await.unwrap();
        let mut events = 0;
        loop {
            let sent = dispatcher.run_once().await.unwrap();
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

/// (người nhận, loại) của hộp thư khách `tenant`.
async fn inbox(tenant: &TenantContext) -> Vec<(Uuid, String)> {
    let connection = tenant.database().read();
    let rows = connection
        .query_all_raw(Statement::from_sql_and_values(
            connection.get_database_backend(),
            if connection.get_database_backend() == sea_orm::DbBackend::Postgres {
                "SELECT user_id, kind FROM notifications_inbox WHERE tenant_id = $1 ORDER BY id"
            } else {
                "SELECT user_id, kind FROM notifications_inbox WHERE tenant_id = ? ORDER BY id"
            },
            [tenant.id().as_uuid().into()],
        ))
        .await
        .unwrap();
    rows.iter()
        .map(|row| {
            (
                row.try_get::<Uuid>("", "user_id").unwrap(),
                row.try_get::<String>("", "kind").unwrap(),
            )
        })
        .collect()
}

#[tokio::test]
async fn thong_bao_nen_tang_toi_quan_tri_vien_moi_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = app.tenant("acme").await.unwrap();
        let beta = app.tenant("beta").await.unwrap();
        let acme_admin = account(&app, &acme, "admin@acme.example", true).await;
        let acme_staff = account(&app, &acme, "staff@acme.example", false).await;
        let beta_admin = account(&app, &beta, "admin@beta.example", true).await;
        let notice = json!({"category": "transactional", "title": "Maintenance", "body": "Sunday 02:00 to 03:00."});

        // Có permission nhưng không phải nhân sự nền tảng: bị chặn.
        let (status, problem) =
            call(&app, Method::POST, "/notifications/platform-notices", "owner", Some(notice.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{problem}");
        assert_eq!(problem["code"], "tenant_platform_only");
        let (status, queued) = call(&app, Method::POST, "/notifications/platform-notices", "ops", Some(notice)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{queued}");
        settle(&app).await;
        settle(&app).await;

        assert_eq!(inbox(&acme).await, [(acme_admin, "notifications.platform_notice".to_owned())]);
        assert!(!inbox(&acme).await.iter().any(|(user, _)| *user == acme_staff));
        assert_eq!(inbox(&beta).await, [(beta_admin, "notifications.platform_notice".to_owned())]);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn thue_bao_bi_tam_khoa_thi_bao_quan_tri_vien_khach() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let acme = app.tenant("acme").await.unwrap();
        let admin = account(&app, &acme, "admin@acme.example", true).await;
        let plan = json!({"code": "pro", "name": "Pro", "status": "active", "grace_days": 7, "suspend_days": 30, "retention_days": 90});
        let (status, plan) = call(&app, Method::POST, "/plans/plans", "ops", Some(plan)).await;
        assert_eq!(status, StatusCode::CREATED, "{plan}");
        let body = json!({"plan_id": plan["data"]["id"], "trial": false});
        let uri = format!("/plans/tenants/{}/subscription", acme.id());
        let (status, sub) = call(&app, Method::PUT, &uri, "ops", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{sub}");
        // Quá hạn và hết ân hạn: vòng đời chuyển sang `suspended`, `plans` báo quản trị viên khách.
        let control = app.registry().control().write();
        control
            .execute_raw(Statement::from_string(
                control.get_database_backend(),
                "UPDATE plans_subscription SET status = 'past_due', grace_ends_at = '2000-01-01T00:00:00Z'",
            ))
            .await
            .unwrap();
        settle(&app).await;
        assert!(
            inbox(&acme).await.contains(&(admin, "plans.subscription_suspended".to_owned())),
            "{:?}",
            inbox(&acme).await
        );
        // Event giao lại / job chạy lại: không gửi trùng.
        settle(&app).await;
        let count = inbox(&acme).await.iter().filter(|(_, kind)| kind == "plans.subscription_suspended").count();
        assert_eq!(count, 1);
    })
    .await
    .unwrap();
}

const PROMO: NotificationKind = NotificationKind::new(
    Category::Marketing,
    &[Channel::Email],
    NotificationTemplate::new("e2e.promo", "Sale", "Everything is on sale.", "Sale"),
);

#[tokio::test]
async fn rut_dong_y_huy_tin_quang_cao_dang_cho() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let notifier = app.api::<dyn Notifier>().unwrap();
        notifier.declare(PROMO).unwrap();
        let ledger = app.api::<dyn ConsentLedger>().unwrap();
        let subject = ConsentSubject::Email("buyer@example.com".to_owned());
        let key = ConsentKey::new("marketing", "", "");
        let evidence = ConsentEvidence {
            text: "Tôi đồng ý nhận tin khuyến mãi.".to_owned(),
            text_version: "1".to_owned(),
            source: "checkout".to_owned(),
            ip_fingerprint: None,
        };
        ledger.grant(DEFAULT_TENANT, subject.clone(), key.clone(), evidence, "e2e:grant").await.unwrap();
        let notification = Notification::new(PROMO.key(), Recipient::Email("buyer@example.com".to_owned()), json!({}), "promo-1");
        let outcome = notifier.notify(DEFAULT_TENANT, notification).await.unwrap();
        assert_eq!(outcome.message_ids.len(), 1, "{outcome:?}");

        // Rút trước khi job gửi chạy: event của `compliance` huỷ tin đang chờ.
        ledger.withdraw(DEFAULT_TENANT, subject, key, "settings", "e2e:withdraw").await.unwrap();
        let dispatcher = app.modules().dispatcher().unwrap();
        while dispatcher.run_once().await.unwrap() > 0 {}
        let connection = app.database().read();
        let row = connection
            .query_one_raw(Statement::from_string(
                connection.get_database_backend(),
                "SELECT status, skip_reason FROM notifications_messages WHERE dedupe_key = 'promo-1'",
            ))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.try_get::<String>("", "status").unwrap(), "canceled");
        assert_eq!(row.try_get::<String>("", "skip_reason").unwrap(), "consent_withdrawn");

        // Quy tắc khung giờ đi kèm `compliance`: SMS quảng cáo 07:00–22:00 giờ Việt Nam.
        let rules = app.api::<dyn ComplianceRules>().unwrap();
        let rule = rules.effective(DEFAULT_TENANT, "notifications.send_windows", chrono::Utc::now()).await.unwrap().unwrap();
        assert_eq!(rule.body["utc_offset_minutes"], 420);
        assert_eq!(rule.body["marketing"]["sms"]["from"], "07:00");
        assert!(rule.legal_basis.contains("91/2020"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn xoa_tai_khoan_thi_xoa_hop_thu_va_tuy_chon() {
    TestDatabase::run(async |test| {
        let mut builder = TestApp::builder(test)
            .setting(
                "AUTOXIDE_MAIL_FILE_DIR",
                &std::env::temp_dir()
                    .join(format!("autoxide-mail-{}", Uuid::now_v7()))
                    .display()
                    .to_string(),
            )
            .user("manager", 1, &["auth.users.manage"]);
        for module in crate::modules::all() {
            builder = builder.boxed_module(module);
        }
        let app = builder.build().await.unwrap();
        let platform = app.default_tenant();
        let user = account(&app, &platform, "leaving@example.com", false).await;
        let notifier = app.api::<dyn Notifier>().unwrap();
        let notice = Notification::new(
            "notifications.platform_notice",
            Recipient::User(user),
            json!({"title": "Hello", "body": "Welcome."}),
            "welcome-1",
        );
        notifier.notify(DEFAULT_TENANT, notice).await.unwrap();
        settle(&app).await;
        assert_eq!(inbox(&platform).await.len(), 1);

        let (status, body) =
            call(&app, Method::DELETE, &format!("/auth/users/{user}"), "manager", None).await;
        assert!(status.is_success(), "{status} {body}");
        settle(&app).await;
        assert!(inbox(&platform).await.is_empty());
    })
    .await
    .unwrap();
}
