//! Vụ việc trên chương trình thật (bước 37, D-363, D-364): người lạ báo hàng giả qua kênh công khai
//! (token chống spam) → `compliance` tự ẩn theo chính sách `cases.auto_restrict` → `content` nghe event
//! và trả 451 → tác giả (bên bị phản ánh, từ sổ đối tượng) phản hồi → quyết định bác → trang hiện lại;
//! vụ trên bình luận ẩn rồi hiện lại; người báo tra cứu bằng mã. `compliance` không đọc bảng của
//! `content` / `comments`. Chạy trên cả SQLite và Postgres.

use autoxide_testkit::{TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

async fn app(test: &TestDatabase) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-cases-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
        .setting("AUTOXIDE_COMMENTS_TARGET_TYPES", "content.entry")
        .setting("AUTOXIDE_COMMENTS_USER_MODERATION", "post")
        .user(
            "admin",
            1,
            &[
                "compliance.cases.read",
                "compliance.cases.handle",
                "compliance.policies.manage",
                "content.read",
                "content.write",
                "content.publish",
                "comments.read",
            ],
        )
        .user("writer", 2, &["content.read", "content.write"])
        .user("reader", 3, &["comments.post"]);
    for module in crate::modules::all() {
        builder = builder.boxed_module(module);
    }
    builder.build().await.unwrap()
}

async fn call(
    app: &TestApp,
    method: Method,
    uri: &str,
    who: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = app.request(method, &format!("/api/v1{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let answer = request.send().await.unwrap();
    let body = if answer.body().is_empty() { Value::Null } else { answer.json() };
    (answer.status(), body)
}

/// Chạy job và phát event tới khi hết việc.
async fn settle(app: &TestApp) {
    let worker = app.modules().worker(8).unwrap().blocked_delay(std::time::Duration::ZERO);
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

async fn page_status(app: &TestApp) -> StatusCode {
    call(app, Method::GET, "/content/public/resolve?locale=en&path=/blog/tui-xach", None, None)
        .await
        .0
}

#[tokio::test]
async fn bao_hang_gia_an_trang_roi_hien_lai_khi_bac() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let (status, entry) = call(
            &app,
            Method::POST,
            "/content/entries",
            Some("writer"),
            Some(json!({"kind": "post", "title": "Tui xach"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{entry}");
        let id = entry["data"]["id"].as_str().unwrap().to_owned();
        let published = app
            .post(&format!("/api/v1/content/entries/{id}/publish"))
            .token("admin")
            .header("if-match", "*")
            .send()
            .await
            .unwrap();
        assert!(published.status().is_success(), "{}", published.text());
        assert_eq!(page_status(&app).await, StatusCode::OK);
        let (status, _) = call(
            &app,
            Method::PUT,
            "/compliance/policies/cases.auto_restrict",
            Some("admin"),
            Some(json!({"value": true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Người lạ báo hàng giả qua kênh công khai.
        let (_, issued) =
            call(&app, Method::GET, "/antispam/token?scope=compliance:reports", None, None).await;
        let report = json!({
            "kind": "counterfeit", "target": format!("content.entry:{id}"), "name": "Lan",
            "email": "lan@example.com", "claim": "This bag is a counterfeit of a registered brand.",
            "form_token": issued["data"]["token"],
        });
        let (status, receipt) =
            call(&app, Method::POST, "/compliance/public/reports", None, Some(report)).await;
        assert_eq!(status, StatusCode::CREATED, "{receipt}");
        let case = receipt["data"]["id"].as_str().unwrap().to_owned();
        let tracking = receipt["data"]["tracking_token"].as_str().unwrap().to_owned();
        settle(&app).await;
        assert_eq!(page_status(&app).await, StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS);

        // Tác giả là bên bị phản ánh (sổ đối tượng của `content`) và phản hồi.
        let (status, mine) =
            call(&app, Method::GET, "/compliance/respondent/cases", Some("writer"), None).await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_eq!(mine["data"][0]["id"], case.as_str());
        let (status, answer) = call(
            &app,
            Method::POST,
            &format!("/compliance/cases/{case}/responses"),
            Some("writer"),
            Some(json!({"body": "Bought from the official distributor; invoice attached."})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{answer}");

        let (status, decided) = call(
            &app,
            Method::POST,
            &format!("/compliance/cases/{case}/decision"),
            Some("admin"),
            Some(json!({"decision": "rejected", "reason": "The invoice shows genuine goods."})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{decided}");
        settle(&app).await;
        assert_eq!(page_status(&app).await, StatusCode::OK);
        let (status, tracked) = call(
            &app,
            Method::GET,
            &format!("/compliance/public/cases/{case}?token={tracking}"),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{tracked}");
        assert_eq!(tracked["data"]["decision"], "rejected");

        // Bình luận bị báo: ẩn rồi hiện lại đúng trạng thái cũ.
        let (status, _) = call(
            &app,
            Method::PUT,
            "/comments/profile",
            Some("reader"),
            Some(json!({"display_name": "Reader"})),
        )
        .await;
        assert!(status.is_success());
        let (status, comment) = call(
            &app,
            Method::POST,
            "/comments/public/comments",
            Some("reader"),
            Some(json!({"target_type": "content.entry", "target_id": id, "body": "Buy fakes at my shop"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{comment}");
        let comment_id = comment["data"]["id"].as_str().unwrap().to_owned();
        let (status, opened) = call(
            &app,
            Method::POST,
            "/compliance/cases",
            Some("admin"),
            Some(json!({
                "kind": "other", "target": format!("comments.comment:{comment_id}"),
                "reporter_kind": "authority", "name": "Market surveillance office",
                "email": "office@example.gov.vn", "claim": "Advertises counterfeit goods.",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{opened}");
        // Yêu cầu của cơ quan: ẩn ngay, quyết định trong 24 giờ; người viết là bên bị phản ánh.
        assert_eq!(opened["data"]["restricted"], true);
        assert_eq!(opened["data"]["respondent_user_ids"], json!([uuid::Uuid::from_u128(3)]));
        settle(&app).await;
        let (_, hidden) =
            call(&app, Method::GET, &format!("/comments/{comment_id}"), Some("admin"), None).await;
        assert_eq!(hidden["data"]["status"], "rejected", "{hidden}");
        let opened_id = opened["data"]["id"].as_str().unwrap().to_owned();
        let (status, _) = call(
            &app,
            Method::POST,
            &format!("/compliance/cases/{opened_id}/restore"),
            Some("admin"),
            Some(json!({"reason": "Checked: no link to a shop."})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        settle(&app).await;
        let (_, shown) =
            call(&app, Method::GET, &format!("/comments/{comment_id}"), Some("admin"), None).await;
        assert_eq!(shown["data"]["status"], "published", "{shown}");
    })
    .await
    .unwrap();
}
