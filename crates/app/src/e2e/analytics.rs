//! Phân tích truy cập (bước 57) trên chương trình thật: website gửi sự kiện qua endpoint công khai,
//! job theo lịch gộp theo ngày, báo cáo đọc số gộp; người dùng đã đồng ý `analytics.personal` trong
//! sổ đồng ý THẬT của `compliance` thì sự kiện gắn người, rút đồng ý (event `compliance.consent_withdrawn`
//! qua bộ phát event) thì gỡ. Chạy trên cả SQLite và Postgres.

use autoxide_compliance::api::{ConsentEvidence, ConsentKey, ConsentLedger, ConsentSubject};
use autoxide_kernel::tenancy::DEFAULT_TENANT;
use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::StatusCode;
use sea_orm::{ConnectionTrait, Statement};
use serde_json::{Value, json};
use uuid::Uuid;

const CHROME: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like \
                      Gecko) Chrome/130.0.0.0 Safari/537.36";

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-analytics-{}", Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .user("ops", 1, &["analytics.read"])
        .user("buyer", 9, &[]);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

/// Gửi một lô như trình duyệt trên `shop.example`.
async fn send(app: &TestApp, token: Option<&str>, events: Value) -> Value {
    let mut request = app
        .post("/api/v1/analytics/public/events")
        .host("shop.example")
        .header("origin", "https://shop.example")
        .header("user-agent", CHROME)
        .json(json!({ "events": events }));
    if let Some(token) = token {
        request = request.token(token);
    }
    let answer = request.send().await.unwrap();
    assert_eq!(answer.status(), StatusCode::ACCEPTED, "{}", answer.text());
    answer.json()["data"].clone()
}

/// Chạy job (lần đầu: mọi lịch) và phát event tới khi hết việc.
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

/// Số sự kiện thô còn gắn người.
async fn linked(app: &TestApp) -> i64 {
    let connection = app.database().read();
    let row = connection
        .query_one_raw(Statement::from_string(
            connection.get_database_backend(),
            "SELECT COUNT(*) AS n FROM analytics_events WHERE user_id IS NOT NULL",
        ))
        .await
        .unwrap()
        .unwrap();
    row.try_get::<i64>("", "n").unwrap()
}

#[tokio::test]
async fn website_gui_su_kien_bao_cao_doc_so_gop_dong_y_that() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let ledger = app.api::<dyn ConsentLedger>().unwrap();
        let subject = ConsentSubject::User(Uuid::from_u128(9));
        let key = ConsentKey::new("analytics.personal", "", "");
        let evidence = ConsentEvidence {
            text: "Tôi đồng ý để website ghi lại các trang tôi xem.".to_owned(),
            text_version: "1".to_owned(),
            source: "web_form".to_owned(),
            ip_fingerprint: None,
        };
        ledger.grant(DEFAULT_TENANT, subject.clone(), key.clone(), evidence, "e2e:grant").await.unwrap();

        let visit = json!([
            {"type": "analytics.page_view", "path": "/", "referrer": "https://www.google.com/"},
            {"type": "analytics.page_view", "path": "/gioi-thieu", "referrer": "https://shop.example/"},
            {"type": "nobody.declared", "path": "/"}
        ]);
        assert_eq!(send(&app, None, visit).await, json!({"accepted": 2, "dropped": 1}));
        let signed_in = json!([{"type": "analytics.page_view", "path": "/tai-khoan"}]);
        assert_eq!(send(&app, Some("buyer"), signed_in).await["accepted"], 1);
        assert_eq!(linked(&app).await, 1);

        // Job theo lịch gộp; báo cáo đọc số gộp.
        settle(&app).await;
        let answer = app.get("/api/v1/analytics/overview").token("ops").send().await.unwrap();
        assert_eq!(answer.status(), StatusCode::OK, "{}", answer.text());
        let overview = answer.json()["data"].clone();
        assert_eq!(overview["views"], 3);
        // Hai nguồn, mỗi nguồn một lượt (bằng nhau thì xếp theo tên): vào thẳng và từ Google.
        let sources: Vec<&str> = overview["top_sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| source["value"].as_str().unwrap())
            .collect();
        assert_eq!(sources, ["direct", "google.com"]);
        let pages = app.get("/api/v1/analytics/pages").token("ops").send().await.unwrap();
        assert_eq!(pages.json()["data"].as_array().unwrap().len(), 3);

        // Rút đồng ý: event của `compliance` tới `analytics`, phần gắn người bị gỡ; số gộp còn.
        ledger.withdraw(DEFAULT_TENANT, subject, key, "settings", "e2e:withdraw").await.unwrap();
        settle(&app).await;
        assert_eq!(linked(&app).await, 0);
        let answer = app.get("/api/v1/analytics/overview").token("ops").send().await.unwrap();
        assert_eq!(answer.json()["data"]["views"], 3);
    })
    .await
    .unwrap();
}
