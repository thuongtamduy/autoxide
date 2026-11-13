//! Test qua HTTP (router thật của kernel, cùng module `antispam` và job worker thật) và MCP
//! (client thật): đăng ký công khai trả lời giống hệt, xác nhận bằng `POST`, gửi chiến dịch cần
//! `If-Match` và `newsletter.send`, huỷ một chạm bằng thân form-urlencoded, trang lựa chọn, xuất /
//! xoá theo email, tool MCP. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::mail::Mailer;
use autoxide_testkit::{TestApp, TestDatabase, TestResponse, tool};
use axum::http::{Method, StatusCode, header};
use serde_json::{Value, json};

use crate::testing::FakeMailer;

struct App {
    http: TestApp,
    mailer: Arc<FakeMailer>,
}

impl App {
    /// Chạy các job tới hạn tới khi hết (như worker của chương trình).
    async fn work(&self) {
        let worker = self.http.modules().worker(self.http.database(), 4).unwrap();
        for _ in 0..20 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
    }
}

/// Token → permission: `admin` mọi quyền; `reader` chỉ đọc; `writer` đọc và soạn; `website` chỉ
/// `newsletter.subscribe` (máy chủ của website gọi đăng ký bằng API key); `member` không quyền
/// nào (tài khoản tự đăng ký). Bên gửi mail là bản giả, thay cho module `mail`.
async fn app(test: &TestDatabase) -> App {
    let mailer = Arc::new(FakeMailer::default());
    let shared: Arc<dyn Mailer> = mailer.clone();
    let http = TestApp::builder(test)
        // Không chờ 3 giây giữa lấy token và gửi.
        .setting("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0")
        .setting("AUTOXIDE_HTTP_PUBLIC_URL", "https://api.example.com")
        .setting("AUTOXIDE_NEWSLETTER_CONFIRM_URL", "https://www.example.com/confirm?token={token}")
        .setting(
            "AUTOXIDE_NEWSLETTER_PREFERENCES_URL",
            "https://www.example.com/prefs?token={token}",
        )
        .setting("AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS", "Example Ltd, Hanoi")
        .user(
            "admin",
            1,
            &["newsletter.read", "newsletter.manage", "newsletter.send", "newsletter.export"],
        )
        .user("reader", 2, &["newsletter.read"])
        .user("writer", 3, &["newsletter.read", "newsletter.manage"])
        .user("website", 4, &["newsletter.subscribe"])
        .user("member", 5, &[])
        .provide::<dyn Mailer>(shared)
        .module(crate::module())
        .build()
        .await
        .unwrap();
    App { http, mailer }
}

/// `code` của lỗi trong thân câu trả lời.
trait Code {
    fn code(&self) -> Option<String>;
}

impl Code for TestResponse {
    fn code(&self) -> Option<String> {
        self.json()["code"].as_str().map(str::to_owned)
    }
}

/// Gửi request tới `/api/v1{uri}`; `who`: token (`None`: không đăng nhập).
async fn send(
    http: &TestApp,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> TestResponse {
    let content_type = body.as_ref().map(|_| "application/json");
    let body = body.map(|body| body.to_string());
    raw(http, method, uri, who, headers, content_type, body).await
}

async fn raw(
    http: &TestApp,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    content_type: Option<&str>,
    body: Option<String>,
) -> TestResponse {
    let method = Method::from_bytes(method.as_bytes()).unwrap();
    let mut request = http.request(method, &format!("/api/v1{uri}"));
    if let Some(who) = who {
        request = request.token(who);
    }
    if let Some(content_type) = content_type {
        request = request.header(header::CONTENT_TYPE.as_str(), content_type);
    }
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(body) = body {
        request = request.bytes(body);
    }
    request.send().await.unwrap()
}

async fn form_token(http: &TestApp) -> String {
    let issued =
        send(http, "GET", "/antispam/token?scope=newsletter:subscribe", None, &[], None).await;
    assert_eq!(issued.status(), StatusCode::OK, "{}", issued.json());
    issued.json()["data"]["token"].as_str().unwrap().to_owned()
}

fn list_body() -> Value {
    json!({
        "key": "weekly",
        "name": {"en": "Weekly news", "vi": "Tin hằng tuần"},
        "description": {"en": "Every Monday."},
        "public": true
    })
}

/// Token trong `…token=<token>`.
fn token_in(text: &str) -> String {
    let start = text.find("token=").unwrap() + "token=".len();
    text[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Vòng đời qua HTTP: danh sách, đăng ký công khai (câu trả lời giống hệt), xác nhận bằng `POST`,
/// chiến dịch (`If-Match`, `newsletter.send`), worker gửi, huỷ một chạm form-urlencoded, trang
/// lựa chọn, xuất / xoá theo email.
#[tokio::test]
async fn vong_doi_qua_http() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let http = &app.http;
        let created = send(http, "POST", "/newsletter/lists", Some("admin"), &[], Some(list_body())).await;
        assert_eq!(created.status(), StatusCode::CREATED, "{}", created.json());
        let list_id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created.header(header::LOCATION), format!("/api/v1/newsletter/lists/{list_id}"));
        let taken = send(http, "POST", "/newsletter/lists", Some("admin"), &[], Some(list_body())).await;
        assert_eq!((taken.status(), taken.code().as_deref()), (StatusCode::CONFLICT, Some("newsletter.list_key_taken")));
        assert_eq!(send(http, "POST", "/newsletter/lists", Some("reader"), &[], Some(list_body())).await.status(), StatusCode::FORBIDDEN);
        let public = send(http, "GET", "/newsletter/public/lists?locale=vi", None, &[], None).await;
        assert_eq!(public.json()["data"]["token_scope"], "newsletter:subscribe");
        assert_eq!(public.json()["data"]["lists"][0]["name"], "Tin hằng tuần");

        // Đăng ký: mới, lặp lại, dính bẫy — cùng mã, cùng thân.
        let body = |token: String, email: &str| json!({"email": email, "lists": ["weekly"], "locale": "vi", "consent_text": "Gửi tôi bản tin.", "form_token": token});
        let first = send(http, "POST", "/newsletter/public/subscribe", None, &[], Some(body(form_token(http).await, "ada@example.com"))).await;
        assert_eq!(first.status(), StatusCode::ACCEPTED, "{}", first.json());
        let again = send(http, "POST", "/newsletter/public/subscribe", None, &[], Some(body(form_token(http).await, "ada@example.com"))).await;
        let mut trapped = body(form_token(http).await, "bot@example.com");
        trapped["trap"] = json!("x");
        let trapped = send(http, "POST", "/newsletter/public/subscribe", None, &[], Some(trapped)).await;
        for other in [&again, &trapped] {
            assert_eq!((other.status(), other.body()), (first.status(), first.body()));
        }
        let missing = send(http, "POST", "/newsletter/public/subscribe", None, &[], Some(json!({"email": "c@example.com", "lists": ["weekly"]}))).await;
        assert_eq!((missing.status(), missing.code().as_deref()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));
        let unknown = send(http, "POST", "/newsletter/public/subscribe", None, &[], Some(json!({"email": "c@example.com", "lists": ["nope"], "form_token": form_token(http).await}))).await;
        assert_eq!((unknown.status(), unknown.json()["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("lists[0]")));
        // Website gọi bằng API key: không cần token.
        let by_website = send(http, "POST", "/newsletter/public/subscribe", Some("website"), &[], Some(json!({"email": "web@example.com", "lists": ["weekly"]}))).await;
        assert_eq!(by_website.status(), StatusCode::ACCEPTED);
        // Đăng nhập mà không có `newsletter.subscribe`: như người lạ, thiếu token thì bị từ chối.
        let by_member = send(http, "POST", "/newsletter/public/subscribe", Some("member"), &[], Some(json!({"email": "member@example.com", "lists": ["weekly"]}))).await;
        assert_eq!((by_member.status(), by_member.code().as_deref()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));

        // Worker lưu các yêu cầu và gửi mail xác nhận (D-154).
        assert!(app.mailer.to("ada@example.com").is_empty());
        app.work().await;

        // Xác nhận: trang của website gọi `POST`.
        let mails = app.mailer.to("ada@example.com");
        assert_eq!(mails.len(), 2);
        let confirm = json!({"token": token_in(&mails[1].text)});
        let confirmed = send(http, "POST", "/newsletter/public/confirm", None, &[], Some(confirm.clone())).await;
        assert_eq!(confirmed.status(), StatusCode::OK, "{}", confirmed.json());
        assert_eq!(confirmed.json()["data"]["lists"][0]["key"], "weekly");
        let reused = send(http, "POST", "/newsletter/public/confirm", None, &[], Some(confirm)).await;
        assert_eq!((reused.status(), reused.code().as_deref()), (StatusCode::BAD_REQUEST, Some("newsletter.confirmation_invalid")));

        // Chiến dịch: người soạn tạo được nhưng không gửi được; gửi cần `If-Match`.
        let draft = json!({"list_id": list_id, "subject": "Hello", "body_text": "News of the week."});
        let campaign = send(http, "POST", "/newsletter/campaigns", Some("writer"), &[], Some(draft.clone())).await;
        assert_eq!(campaign.status(), StatusCode::CREATED, "{}", campaign.json());
        assert_eq!(send(http, "POST", "/newsletter/campaigns", Some("reader"), &[], Some(draft)).await.status(), StatusCode::FORBIDDEN);
        let id = campaign.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(http, "GET", &format!("/newsletter/campaigns/{id}"), Some("reader"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let path = format!("/newsletter/campaigns/{id}/send");
        assert_eq!(send(http, "POST", &path, Some("admin"), &[], None).await.status(), StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(send(http, "POST", &path, Some("writer"), &[("if-match", &etag)], None).await.status(), StatusCode::FORBIDDEN);
        let edit = send(http, "PATCH", &format!("/newsletter/campaigns/{id}"), Some("writer"), &[("if-match", &etag)], Some(json!({"subject": "Hello again"}))).await;
        assert_eq!(edit.status(), StatusCode::OK, "{}", edit.json());
        assert_eq!(send(http, "POST", &path, Some("admin"), &[("if-match", &etag)], None).await.status(), StatusCode::PRECONDITION_FAILED);
        let fresh = edit.header(header::ETAG).to_owned();
        let started = send(http, "POST", &path, Some("admin"), &[("if-match", &fresh)], None).await;
        assert_eq!((started.status(), started.json()["data"]["status"].as_str()), (StatusCode::OK, Some("sending")), "{}", started.json());
        app.work().await;
        let mail = app.mailer.to("ada@example.com").pop().unwrap();
        assert_eq!(mail.subject, "Hello again");
        let header = mail.headers.iter().find(|(name, _)| name == "List-Unsubscribe").unwrap().1.clone();
        let progress = send(http, "GET", &format!("/newsletter/campaigns/{id}"), Some("reader"), &[], None).await;
        assert_eq!(progress.json()["data"]["progress"]["sent"], 1, "{}", progress.json());

        // Trang lựa chọn, rồi huỷ một chạm như ứng dụng mail (thân form-urlencoded).
        let token = token_in(&header);
        let prefs = send(http, "GET", &format!("/newsletter/public/preferences?token={token}"), None, &[], None).await;
        assert_eq!((prefs.status(), prefs.json()["data"]["lists"][0]["subscribed"].as_bool()), (StatusCode::OK, Some(true)), "{}", prefs.json());
        let one_click = raw(http, "POST", &format!("/newsletter/public/unsubscribe?token={token}"), None, &[], Some("application/x-www-form-urlencoded"), Some("List-Unsubscribe=One-Click".to_owned())).await;
        assert_eq!(one_click.status(), StatusCode::OK, "{}", String::from_utf8_lossy(one_click.body()));
        let unknown = raw(http, "POST", "/newsletter/public/unsubscribe?token=nope", None, &[], Some("application/x-www-form-urlencoded"), Some("List-Unsubscribe=One-Click".to_owned())).await;
        assert_eq!((unknown.status(), unknown.body()), (one_click.status(), one_click.body()));
        let left = send(http, "GET", &format!("/newsletter/subscribers?list_id={list_id}&status=unsubscribed"), Some("reader"), &[], None).await;
        assert_eq!(left.json()["data"][0]["email"], "ada@example.com", "{}", left.json());
        let back = send(http, "PUT", &format!("/newsletter/public/preferences?token={token}"), None, &[], Some(json!({"lists": [{"key": "weekly", "subscribed": true}]}))).await;
        assert_eq!(back.json()["data"]["lists"][0]["status"], "confirmed", "{}", back.json());
        assert_eq!(send(http, "GET", "/newsletter/public/preferences?token=nope", None, &[], None).await.status(), StatusCode::NOT_FOUND);

        // Danh sách: sửa cần `If-Match`.
        assert_eq!(send(http, "PATCH", &format!("/newsletter/lists/{list_id}"), Some("admin"), &[], Some(json!({"public": false}))).await.status(), StatusCode::PRECONDITION_REQUIRED);

        // Bounce, danh sách chặn, xuất / xoá theo email.
        let bounce = send(http, "POST", "/newsletter/bounces", Some("admin"), &[], Some(json!({"email": "web@example.com", "type": "complaint"}))).await;
        assert_eq!(bounce.json()["data"]["suppressed"], true, "{}", bounce.json());
        let suppressions = send(http, "GET", "/newsletter/suppressions", Some("reader"), &[], None).await;
        assert_eq!(suppressions.json()["data"][0]["reason"], "complaint");
        let data = send(http, "GET", "/newsletter/data?email=ADA@example.com", Some("admin"), &[], None).await;
        assert_eq!(data.status(), StatusCode::OK, "{}", data.json());
        assert_eq!(data.json()["data"]["deliveries"].as_array().unwrap().len(), 1);
        assert_eq!(send(http, "GET", "/newsletter/data?email=ada@example.com", Some("reader"), &[], None).await.status(), StatusCode::FORBIDDEN);
        assert_eq!(send(http, "GET", "/newsletter/data", Some("admin"), &[], None).await.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let erased = send(http, "DELETE", "/newsletter/data?email=ada@example.com", Some("admin"), &[], None).await;
        assert_eq!(erased.json()["data"]["deleted"], true);
        let after = send(http, "GET", "/newsletter/data?email=ada@example.com", Some("admin"), &[], None).await;
        assert_eq!(after.json()["data"]["subscriber"], Value::Null);
    })
    .await
    .unwrap();
}

/// Tool MCP qua client thật: người chỉ đọc chỉ thấy tool đọc; người soạn tạo được chiến dịch
/// nháp nhưng không gửi được; gửi cần `version` đúng.
#[tokio::test]
async fn tool_mcp() {
    TestDatabase::run(async |test| {
        let app = app(test).await;
        let created =
            send(&app.http, "POST", "/newsletter/lists", Some("admin"), &[], Some(list_body()))
                .await;
        let list_id = created.json()["data"]["id"].clone();
        let client = async |who: &str| app.http.mcp(who).await.unwrap();
        let names = async |who: &str| {
            let connected = client(who).await;
            let mut names: Vec<String> = connected
                .list_all_tools()
                .await
                .unwrap()
                .into_iter()
                .map(|tool| tool.name.to_string())
                .collect();
            names.sort();
            connected.cancel().await.unwrap();
            names
        };
        assert_eq!(
            names("reader").await,
            ["newsletter.find_subscribers", "newsletter.get_campaign", "newsletter.list_lists"]
        );
        assert_eq!(names("writer").await.len(), 4);

        let admin = client("admin").await;
        let tools = admin.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 6);
        for tool in &tools {
            assert!(
                !serde_json::to_string(&tool.input_schema).unwrap().contains("$ref"),
                "{} has a $ref",
                tool.name
            );
        }
        let call = async |name: &str, arguments: Value| {
            let result = admin.call_tool(tool(name, arguments)).await.unwrap();
            (result.is_error == Some(true), result.structured_content.unwrap_or(Value::Null))
        };
        let (failed, lists) = call("newsletter.list_lists", json!({})).await;
        assert!(!failed, "{lists}");
        assert_eq!(lists["data"][0]["key"], "weekly");
        let (failed, draft) = call(
            "newsletter.create_campaign",
            json!({"list_id": list_id, "subject": "Hi", "body_text": "Hello"}),
        )
        .await;
        assert!(!failed, "{draft}");
        let id = draft["data"]["id"].clone();
        let version = draft["data"]["version"].as_str().unwrap().to_owned();
        let (missing, _) = call("newsletter.send_campaign", json!({"id": id})).await;
        assert!(missing, "`version` is required");
        let (stale, _) =
            call("newsletter.send_campaign", json!({"id": id, "version": "\"1\""})).await;
        assert!(stale, "an old version is refused");
        let (failed, sent) =
            call("newsletter.send_campaign", json!({"id": id, "version": version})).await;
        assert!(!failed, "{sent}");
        assert_eq!(sent["data"]["status"], "sending");
        let (_, progress) = call("newsletter.get_campaign", json!({"id": id})).await;
        assert_eq!(progress["data"]["status"], "sending");
        let (_, found) = call("newsletter.find_subscribers", json!({"email": "nobody"})).await;
        assert_eq!(found["data"], json!([]));
        admin.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
