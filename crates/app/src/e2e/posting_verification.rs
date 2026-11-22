//! Xác thực tài khoản đăng nội dung (NĐ 147/2024, D-202, D-356) trên chương trình thật: `compliance`
//! đăng ký bộ hỏi `PostingVerifier` của kernel, `comments` hỏi nó ở loại đích chỉ cho tài khoản đã
//! xác thực (`AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES`) — chưa xác thực số điện thoại bị từ chối, xác
//! thực bằng mã SMS (bản giả) xong thì đăng được; chính sách `posting.comments` của khách khác `none`
//! thì mọi loại đích đều cần xác thực. Chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::sms::SmsSender;
use autoxide_testkit::{FakeSmsSender, TestApp, TestDatabase};
use axum::http::{Method, StatusCode};
use serde_json::{Value, json};

/// Loại đích chỉ tài khoản đã xác thực; loại kia mở cho mọi tài khoản.
const QUESTIONS: &str = "demo.question";
const OPEN: &str = "demo.page";

async fn app(test: &TestDatabase, sms: Arc<FakeSmsSender>) -> TestApp {
    let dir = std::env::temp_dir().join(format!("autoxide-posting-{}", uuid::Uuid::now_v7()));
    let mut builder = TestApp::builder(test)
        .setting("AUTOXIDE_FILES_DIR", &dir.join("files").display().to_string())
        .setting("AUTOXIDE_MAIL_FILE_DIR", &dir.join("mail").display().to_string())
        .setting("AUTOXIDE_COMMENTS_TARGET_TYPES", &format!("{QUESTIONS},{OPEN}"))
        .setting("AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES", QUESTIONS)
        .setting("AUTOXIDE_COMMENTS_USER_MODERATION", "post")
        .provide::<dyn SmsSender>(sms)
        .user("admin", 1, &["compliance.policies.manage"])
        .user("alice", 2, &["comments.post"])
        .user("bob", 3, &["comments.post"]);
    // Bài này cấp bộ gửi SMS giả; bộ nối thật `esms` cũng công bố `dyn SmsSender` (một hợp đồng chỉ có
    // một bên cung cấp) nên không nạp ở đây.
    for module in crate::modules::all().into_iter().filter(|module| module.name() != "esms") {
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

async fn comment(app: &TestApp, who: &str, kind: &str, body: &str) -> (StatusCode, Value) {
    let input = json!({"target_type": kind, "target_id": "1", "body": body});
    call(app, Method::POST, "/comments/public/comments", who, Some(input)).await
}

/// Xác thực số `phone` cho `who` bằng mã trong tin SMS giả.
async fn verify_phone(app: &TestApp, sms: &FakeSmsSender, who: &str, phone: &str) {
    let (status, sent) = call(
        app,
        Method::POST,
        "/compliance/me/phone-verifications",
        who,
        Some(json!({"phone": phone})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{sent}");
    let id = sent["data"]["id"].as_str().unwrap();
    let e164 = format!("+84{}", phone.trim_start_matches('0').replace(' ', ""));
    let text = sms.last_text(&e164).unwrap();
    let code = text.split(|c: char| !c.is_ascii_digit()).find(|part| part.len() == 6).unwrap();
    let (status, done) = call(
        app,
        Method::POST,
        &format!("/compliance/me/phone-verifications/{id}/confirm"),
        who,
        Some(json!({"code": code})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["data"]["level"], "phone");
}

#[tokio::test]
async fn binh_luan_chi_cho_tai_khoan_da_xac_thuc_so_dien_thoai() {
    TestDatabase::run(async |test| {
        let sms = Arc::new(FakeSmsSender::new());
        let app = app(test, sms.clone()).await;
        for (who, name) in [("alice", "Alice"), ("bob", "Bob")] {
            let (status, body) = call(
                &app,
                Method::PUT,
                "/comments/profile",
                who,
                Some(json!({"display_name": name})),
            )
            .await;
            assert!(status.is_success(), "{body}");
        }

        // Loại đích chỉ cho tài khoản đã xác thực: chưa xác thực số → 403.
        let (status, refused) = comment(&app, "alice", QUESTIONS, "Hàng còn không?").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert_eq!(refused["code"], "comments.verification_required");
        // Loại đích thường: đăng được.
        let (status, body) = comment(&app, "alice", OPEN, "Bài hay").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        verify_phone(&app, &sms, "alice", "0912 345 678").await;
        let (status, body) = comment(&app, "alice", QUESTIONS, "Hàng còn không?").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // Khách đặt `posting.comments = phone`: mọi loại đích cần xác thực.
        let (status, body) = call(
            &app,
            Method::PUT,
            "/compliance/policies/posting.comments",
            "admin",
            Some(json!({"value": "phone"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, refused) = comment(&app, "bob", OPEN, "Bài hay").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
        assert_eq!(refused["code"], "comments.verification_required");
        let (status, body) = comment(&app, "alice", OPEN, "Bài hay lắm").await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // `identity`: số điện thoại không còn đủ.
        let (status, _) = call(
            &app,
            Method::PUT,
            "/compliance/policies/posting.comments",
            "admin",
            Some(json!({"value": "identity"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, refused) = comment(&app, "alice", QUESTIONS, "Còn size M?").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

        // Gỡ số đã xác thực, về `none` cho bình luận: loại đích chỉ cho tài khoản đã xác thực lại chặn.
        let (status, _) = call(
            &app,
            Method::PUT,
            "/compliance/policies/posting.comments",
            "admin",
            Some(json!({"value": "none"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = call(&app, Method::DELETE, "/compliance/me/phone", "alice", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = comment(&app, "alice", QUESTIONS, "Còn size L?").await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = comment(&app, "alice", OPEN, "Bài hay nữa").await;
        assert_eq!(status, StatusCode::CREATED);
    })
    .await
    .unwrap();
}
