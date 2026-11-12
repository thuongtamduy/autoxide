//! Test qua HTTP (router thật của kernel, cùng module `antispam` thật, một module `content` giả
//! đăng ký loại đích và khai event) và MCP (client thật): câu trả lời công khai giống hệt cho bài
//! bị giữ / dính bẫy / quá nhiều lần, gửi lại cùng token, phần công khai không lộ email hay dấu
//! vết IP, XSS, `If-Match` bắt buộc, sửa / xoá bằng token sửa, báo cáo, cấm, đích, xuất / xoá
//! theo email, giới hạn thân request, tool MCP. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::error::Error;
use autoxide_kernel::event::EventInfo;
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_testkit::TestDatabase;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::testing::{Entries, FakeMailer};

#[cfg(test)]
mod fake {
    //! Bộ xác thực giả (chỉ có trong test; `cargo xtask check` bỏ qua phần `#[cfg(test)]`).

    use autoxide_kernel::access::{Authenticator, Principal};
    use autoxide_kernel::error::Error;
    use uuid::Uuid;

    /// Token → permission: `admin` mọi quyền của module; `reader` chỉ đọc; `member` bình luận
    /// khi đăng nhập.
    pub(super) struct FakeAuthenticator;

    #[async_trait::async_trait]
    impl Authenticator for FakeAuthenticator {
        async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
            let (id, permissions): (u128, &[&str]) = match authorization {
                Some("Bearer admin") => (
                    1,
                    &[
                        "comments.post",
                        "comments.read",
                        "comments.moderate",
                        "comments.manage",
                        "comments.export",
                    ],
                ),
                Some("Bearer reader") => (2, &["comments.read"]),
                Some("Bearer member") => (3, &["comments.post"]),
                _ => {
                    return Err(Error::unauthenticated(
                        "auth.invalid_token",
                        "The token is not valid.",
                    ));
                }
            };
            Ok(Principal::new(Uuid::from_u128(id), permissions.iter().map(|p| (*p).to_owned())))
        }
    }
}

use fake::FakeAuthenticator;

/// Module giả công bố bên gửi mail, thay cho module `mail`.
struct TestMail(Arc<FakeMailer>);

impl Module for TestMail {
    fn name(&self) -> &'static str {
        "testmail"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let mailer: Arc<dyn Mailer> = self.0.clone();
        context.provide::<dyn Mailer>(mailer)
    }
}

/// Module `content` giả: đăng ký loại đích `content.entry` và khai hai event mà `comments` nghe.
struct TestContent;

impl Module for TestContent {
    fn name(&self) -> &'static str {
        "content"
    }

    fn events(&self) -> Vec<EventInfo> {
        vec![
            EventInfo::new("content.entry_archived", "Archived."),
            EventInfo::new("content.entry_deleted", "Deleted."),
        ]
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        context.resource_type("content.entry", Arc::new(Entries))
    }
}

struct App {
    router: Router,
    modules: Modules,
}

async fn app(test: &TestDatabase) -> App {
    let source = ConfigSource::from_pairs([
        ("DATABASE_URL", test.url()),
        // Không chờ 3 giây giữa lấy token và gửi.
        ("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0"),
        ("AUTOXIDE_COMMENTS_PER_EMAIL_PER_HOUR", "3"),
        ("AUTOXIDE_COMMENTS_VERIFY_URL", "https://www.example.com/verify?token={token}"),
        ("AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL", "https://www.example.com/stop?token={token}"),
    ]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let mailer = Arc::new(FakeMailer::default());
    let mut modules = Modules::new(vec![
        Box::new(TestMail(mailer)),
        Box::new(TestContent),
        Box::new(crate::module()),
    ])
    .unwrap();
    let configured = modules.configure(&mut reader);
    let (config, _) = reader.finish(config.and_then(|config| configured.map(|_| config))).unwrap();
    let database = autoxide_kernel::database::Database::connect(&config.database).await.unwrap();
    modules.migrator().unwrap().up(&database).await.unwrap();
    modules.start(&database).unwrap();
    let extras = autoxide_kernel::http::Extras {
        authenticator: Some(Arc::new(FakeAuthenticator)),
        tools: modules.tools().unwrap(),
        translations: modules.translations().clone(),
        body_limits: modules.body_limits(),
        ..Default::default()
    };
    let router = autoxide_kernel::http::build(
        &config.http,
        utoipa_axum::router::OpenApiRouter::new(),
        modules.routes(),
        extras,
    )
    .router;
    App { router, modules }
}

struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    fn header(&self, name: header::HeaderName) -> &str {
        self.headers.get(name).map_or("", |value| value.to_str().unwrap())
    }

    fn text(&self) -> String {
        String::from_utf8(self.body.clone()).unwrap()
    }

    fn code(&self) -> Option<String> {
        self.json()["code"].as_str().map(str::to_owned)
    }
}

/// Gửi request tới `/api/v1{uri}`; `who`: token (`None`: không đăng nhập).
async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(format!("/api/v1{uri}"));
    if let Some(who) = who {
        request = request.header("authorization", format!("Bearer {who}"));
    }
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let response = router.clone().oneshot(request.body(body).unwrap()).await;
    let (parts, body) = response.unwrap().into_parts();
    let body = axum::body::to_bytes(body, usize::MAX).await.unwrap().to_vec();
    Answer { status: parts.status, headers: parts.headers, body }
}

async fn form_token(router: &Router) -> String {
    let issued =
        send(router, "GET", "/antispam/token?scope=comments:content.entry", None, &[], None).await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.json());
    issued.json()["data"]["token"].as_str().unwrap().to_owned()
}

/// Các khoá của `data` trong câu trả lời, để so hình dạng.
fn shape(answer: &Answer) -> Vec<String> {
    let mut keys: Vec<String> =
        answer.json()["data"].as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn comment(token: &str, name: &str, email: &str, body: &str) -> Value {
    json!({
        "target_type": "content.entry", "target_id": "public-1",
        "author_name": name, "email": email, "body": body, "form_token": token
    })
}

/// Vòng đời qua HTTP.
#[tokio::test]
async fn vong_doi_qua_http() {
    TestDatabase::run(async |test| {
        let App { router, .. } = app(test).await;
        let settings = send(&router, "GET", "/comments/public/settings", None, &[], None).await;
        assert_eq!(settings.status, StatusCode::OK);
        assert_eq!(settings.json()["data"]["reply_mails"], true);
        assert_eq!(settings.json()["data"]["guests_allowed"], true);

        // Bài thật của khách: chờ duyệt (mặc định `pre`), có token sửa, `Location`.
        let xss = "<script>alert(1)</script> <img src=x onerror=alert(1)> javascript:alert(1) https://ok.example";
        let token = form_token(&router).await;
        let held = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&token, "Ada", "ada@example.com", xss))).await;
        assert_eq!(held.status, StatusCode::CREATED, "{}", held.json());
        let id = held.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(held.header(header::LOCATION), format!("/api/v1/comments/public/comments/{id}"));
        assert_eq!(held.json()["data"]["status"], "pending");
        for private in ["ada@example.com", "ip_fingerprint", "hold_reason", "moderation"] {
            assert!(!held.text().contains(private), "the answer shows `{private}`");
        }
        // Gửi lại cùng token: cùng bình luận.
        let again = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&token, "Ada", "ada@example.com", xss))).await;
        assert_eq!((again.status, again.json()["data"]["id"].as_str()), (StatusCode::CREATED, Some(id.as_str())));
        // Token sửa mới thay token cũ (máy chủ chỉ giữ bản băm).
        let edit_token = again.json()["data"]["edit_token"].as_str().unwrap().to_owned();
        assert_ne!(Some(edit_token.as_str()), held.json()["data"]["edit_token"].as_str());

        // Dính bẫy, quá nhiều lần với một email (giới hạn 3/giờ; gửi lại cùng token KHÔNG đếm,
        // để không ai khoá được email của người khác — D-150): câu trả lời y hệt.
        let mut trapped = comment(&form_token(&router).await, "Bot", "bot@example.com", "spam");
        trapped["trap"] = json!("http://spam.example");
        let trapped = send(&router, "POST", "/comments/public/comments", None, &[], Some(trapped)).await;
        assert_eq!((trapped.status, shape(&trapped), &trapped.json()["data"]["status"]), (held.status, shape(&held), &held.json()["data"]["status"]));
        assert_eq!(trapped.header(header::LOCATION).len(), held.header(header::LOCATION).len());
        let second = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&form_token(&router).await, "Ada", "ada@example.com", "Lần hai"))).await;
        assert_eq!(second.status, StatusCode::CREATED);
        let third = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&form_token(&router).await, "Ada", "ada@example.com", "Lần ba"))).await;
        assert_eq!(third.status, StatusCode::CREATED);
        let limited = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&form_token(&router).await, "Ada", "ada@example.com", "Lần bốn"))).await;
        assert_eq!((limited.status, shape(&limited), &limited.json()["data"]["status"]), (held.status, shape(&held), &held.json()["data"]["status"]));

        // Lỗi thật thì rõ ràng.
        let no_token = json!({"target_type": "content.entry", "target_id": "public-1", "author_name": "A", "body": "x"});
        let missing = send(&router, "POST", "/comments/public/comments", None, &[], Some(no_token)).await;
        assert_eq!((missing.status, missing.code().as_deref()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));
        let invalid = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&form_token(&router).await, "", "nope", ""))).await;
        assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(invalid.json()["errors"].as_array().unwrap().len(), 3);
        let unknown = send(&router, "POST", "/comments/public/comments", None, &[], Some(json!({"target_type": "forms.form", "target_id": "1", "author_name": "A", "body": "x", "form_token": "t"}))).await;
        assert_eq!((unknown.status, unknown.code().as_deref()), (StatusCode::NOT_FOUND, Some("comments.target_not_found")));
        let huge = comment("t", "A", "a@example.com", &"x".repeat(200_000));
        assert_eq!(send(&router, "POST", "/comments/public/comments", None, &[], Some(huge)).await.status, StatusCode::PAYLOAD_TOO_LARGE);

        // Chưa duyệt: không có gì công khai.
        let thread_uri = "/comments/public/threads?target_type=content.entry&target_id=public-1";
        let empty = send(&router, "GET", thread_uri, None, &[], None).await;
        assert_eq!(empty.json()["data"], json!([]));
        assert_eq!(send(&router, "GET", &format!("/comments/public/comments/{id}"), None, &[], None).await.status, StatusCode::NOT_FOUND);

        // Hàng chờ và duyệt: `If-Match` bắt buộc.
        let queue = send(&router, "GET", "/comments?q=ok.example", Some("reader"), &[], None).await;
        assert_eq!(queue.status, StatusCode::OK, "{}", queue.json());
        assert_eq!(queue.json()["data"][0]["author_email"], "ada@example.com");
        assert_eq!(send(&router, "GET", "/comments", None, &[], None).await.status, StatusCode::UNAUTHORIZED);
        let one = send(&router, "GET", &format!("/comments/{id}"), Some("reader"), &[], None).await;
        let etag = one.header(header::ETAG).to_owned();
        let approve = json!({"status": "published"});
        assert_eq!(send(&router, "PATCH", &format!("/comments/{id}"), Some("admin"), &[], Some(approve.clone())).await.status, StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(send(&router, "PATCH", &format!("/comments/{id}"), Some("reader"), &[("if-match", &etag)], Some(approve.clone())).await.status, StatusCode::FORBIDDEN);
        let approved = send(&router, "PATCH", &format!("/comments/{id}"), Some("admin"), &[("if-match", &etag)], Some(approve.clone())).await;
        assert_eq!((approved.status, approved.json()["data"]["status"].as_str()), (StatusCode::OK, Some("published")));
        assert_eq!(send(&router, "PATCH", &format!("/comments/{id}"), Some("admin"), &[("if-match", &etag)], Some(json!({"status": "spam"}))).await.status, StatusCode::PRECONDITION_FAILED);

        // Công khai: HTML an toàn, không email / dấu vết IP.
        let thread = send(&router, "GET", thread_uri, None, &[], None).await;
        assert_eq!(thread.status, StatusCode::OK);
        let html = thread.json()["data"][0]["body_html"].as_str().unwrap().to_owned();
        assert!(!html.contains("<script") && !html.contains("<img") && !html.contains("href=\"javascript"), "{html}");
        assert!(html.contains("<a href=\"https://ok.example\" rel=\"nofollow ugc noopener\">"), "{html}");
        assert_eq!(thread.json()["data"][0]["author"], json!({"kind": "guest", "name": "Ada"}));
        for private in ["ada@example.com", "ip_fingerprint", "hold_reason", "author_email"] {
            assert!(!thread.text().contains(private), "the thread shows `{private}`");
        }
        let counts = send(&router, "GET", "/comments/public/counts?target_type=content.entry&target_ids=public-1,public-2", None, &[], None).await;
        assert_eq!(counts.json()["data"], json!([{"target_id": "public-1", "count": 1}, {"target_id": "public-2", "count": 0}]));
        let target = send(&router, "GET", "/comments/public/targets/content.entry/public-1", None, &[], None).await;
        assert_eq!((target.json()["data"]["state"].as_str(), target.json()["data"]["comment_count"].as_i64(), &target.json()["data"]["pending_count"]), (Some("open"), Some(1), &Value::Null));

        // Sửa và xoá bằng token sửa; báo cáo.
        let edit = send(&router, "PATCH", &format!("/comments/public/comments/{id}"), None, &[], Some(json!({"body": "Đã sửa", "edit_token": edit_token}))).await;
        assert_eq!((edit.status, edit.json()["data"]["body"].as_str(), edit.json()["data"]["status"].as_str()), (StatusCode::OK, Some("Đã sửa"), Some("pending")));
        // Khách ở chế độ `pre`: bản sửa chờ duyệt lại.
        let etag = send(&router, "GET", &format!("/comments/{id}"), Some("reader"), &[], None).await.header(header::ETAG).to_owned();
        assert_eq!(send(&router, "PATCH", &format!("/comments/{id}"), Some("admin"), &[("if-match", &etag)], Some(approve.clone())).await.status, StatusCode::OK);
        let wrong = send(&router, "PATCH", &format!("/comments/public/comments/{id}"), None, &[], Some(json!({"body": "x", "edit_token": "nope"}))).await;
        assert_eq!((wrong.status, wrong.code().as_deref()), (StatusCode::FORBIDDEN, Some("comments.not_author")));
        let report = send(&router, "POST", &format!("/comments/public/comments/{id}/reports"), None, &[], Some(json!({"reason": "abuse", "note": "rude"}))).await;
        assert_eq!((report.status, report.json()["data"]["received"].as_bool()), (StatusCode::OK, Some(true)));
        let reports = send(&router, "GET", &format!("/comments/{id}/reports"), Some("reader"), &[], None).await;
        assert_eq!(reports.json()["data"][0]["reason"], "abuse");

        // Người dùng: tên hiển thị trước, lần đầu chờ duyệt.
        let body = json!({"target_type": "content.entry", "target_id": "public-1", "body": "Của thành viên"});
        let no_name = send(&router, "POST", "/comments/public/comments", Some("member"), &[], Some(body.clone())).await;
        assert_eq!((no_name.status, no_name.code().as_deref()), (StatusCode::CONFLICT, Some("comments.profile_required")));
        let named = send(&router, "PUT", "/comments/profile", Some("member"), &[], Some(json!({"display_name": "Member One"}))).await;
        assert_eq!((named.status, named.json()["data"]["display_name"].as_str()), (StatusCode::OK, Some("Member One")));
        assert_eq!(send(&router, "GET", "/comments/profile", Some("reader"), &[], None).await.status, StatusCode::FORBIDDEN);
        let mine = send(&router, "POST", "/comments/public/comments", Some("member"), &[], Some(body)).await;
        assert_eq!((mine.status, mine.json()["data"]["status"].as_str(), &mine.json()["data"]["edit_token"]), (StatusCode::CREATED, Some("pending"), &Value::Null));
        let mine = mine.json()["data"]["id"].as_str().unwrap().to_owned();
        let deleted = send(&router, "POST", &format!("/comments/public/comments/{mine}/delete"), Some("member"), &[], Some(json!({}))).await;
        assert_eq!(deleted.status, StatusCode::OK, "{}", deleted.json());

        // Cấm.
        let ban = send(&router, "POST", "/comments/bans", Some("admin"), &[], Some(json!({"kind": "email", "value": "bad@example.com", "reason": "Spam"}))).await;
        assert_eq!(ban.status, StatusCode::CREATED, "{}", ban.json());
        let ban_id = ban.json()["data"]["id"].as_str().unwrap().to_owned();
        let banned = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&form_token(&router).await, "Bad", "bad@example.com", "hi"))).await;
        assert_eq!((banned.status, banned.code().as_deref()), (StatusCode::FORBIDDEN, Some("comments.banned")));
        let bans = send(&router, "GET", "/comments/bans?active=true", Some("admin"), &[], None).await;
        assert_eq!(bans.json()["data"].as_array().unwrap().len(), 1);
        assert_eq!(send(&router, "GET", "/comments/bans", Some("reader"), &[], None).await.status, StatusCode::FORBIDDEN);
        assert_eq!(send(&router, "DELETE", &format!("/comments/bans/{ban_id}"), Some("admin"), &[], None).await.status, StatusCode::NO_CONTENT);

        // Đóng đích.
        let closed = send(&router, "PUT", "/comments/targets/content.entry/public-1", Some("admin"), &[], Some(json!({"state": "closed"}))).await;
        assert_eq!((closed.status, closed.json()["data"]["state"].as_str()), (StatusCode::OK, Some("closed")));
        let late = send(&router, "POST", "/comments/public/comments", None, &[], Some(comment(&form_token(&router).await, "Cid", "cid@example.com", "Muộn"))).await;
        assert_eq!((late.status, late.code().as_deref()), (StatusCode::CONFLICT, Some("comments.target_closed")));
        let admin_target = send(&router, "GET", "/comments/targets/content.entry/public-1", Some("reader"), &[], None).await;
        assert!(!admin_target.header(header::ETAG).is_empty());
        assert!(admin_target.json()["data"]["pending_count"].is_number());

        // Xác minh / huỷ theo dõi bằng token lạ.
        let bad = send(&router, "POST", "/comments/public/verify", None, &[], Some(json!({"token": "nope"}))).await;
        assert_eq!((bad.status, bad.code().as_deref()), (StatusCode::BAD_REQUEST, Some("comments.link_invalid")));
        let stop = send(&router, "POST", "/comments/public/unsubscribe", None, &[], Some(json!({"token": "nope"}))).await;
        assert_eq!(stop.json()["data"]["unsubscribed"], true);

        // Xuất / xoá theo email; xoá hẳn.
        let data = send(&router, "GET", "/comments/data?email=ADA@example.com", Some("admin"), &[], None).await;
        assert_eq!(data.status, StatusCode::OK, "{}", data.json());
        assert_eq!(data.json()["data"]["comments"].as_array().unwrap().len(), 3);
        assert_eq!(send(&router, "GET", "/comments/data?email=ada@example.com", Some("reader"), &[], None).await.status, StatusCode::FORBIDDEN);
        let erased = send(&router, "DELETE", "/comments/data?email=ada@example.com", Some("admin"), &[], None).await;
        assert_eq!(erased.json()["data"]["cleared"], 3);
        let purged = send(&router, "DELETE", &format!("/comments/{id}"), Some("admin"), &[], None).await;
        assert_eq!(purged.json()["data"]["deleted"], 1);
        assert_eq!(send(&router, "GET", &format!("/comments/{id}"), Some("admin"), &[], None).await.status, StatusCode::NOT_FOUND);
    })
    .await
    .unwrap();
}

/// Tool MCP qua client thật: người chỉ đọc chỉ thấy tool đọc; duyệt cần `version` đúng; đóng đích.
#[tokio::test]
async fn tool_mcp() {
    TestDatabase::run(async |test| {
        let App { router, modules } = app(test).await;
        assert!(modules.names().contains(&"comments"));
        let token = form_token(&router).await;
        let sent = send(
            &router,
            "POST",
            "/comments/public/comments",
            None,
            &[],
            Some(comment(&token, "Ada", "ada@example.com", "Chờ duyệt")),
        )
        .await;
        let id = sent.json()["data"]["id"].clone();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        autoxide_kernel::tls::install_crypto_provider();
        let client = async |who: &str| {
            let config = StreamableHttpClientTransportConfig::with_uri(url.clone())
                .auth_header(who.to_owned());
            ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap()
        };
        let reader = client("reader").await;
        let mut names: Vec<String> = reader
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["comments.get", "comments.queue"]);
        reader.cancel().await.unwrap();

        let admin = client("admin").await;
        let tools = admin.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 4);
        for tool in &tools {
            assert!(
                !serde_json::to_string(&tool.input_schema).unwrap().contains("$ref"),
                "{} has a $ref",
                tool.name
            );
        }
        let call = async |name: &str, arguments: Value| {
            let request = CallToolRequestParams::new(name.to_owned())
                .with_arguments(arguments.as_object().cloned().unwrap());
            let result = admin.call_tool(request).await.unwrap();
            (result.is_error == Some(true), result.structured_content.unwrap_or(Value::Null))
        };
        let (failed, queue) = call("comments.queue", json!({"q": "cho duyet"})).await;
        assert!(!failed, "{queue}");
        assert_eq!(queue["data"][0]["id"], id);
        let (_, one) = call("comments.get", json!({"id": id})).await;
        let version = one["data"]["version"].as_str().unwrap().to_owned();
        assert_eq!(one["data"]["author_email"], "ada@example.com");
        let (missing, _) =
            call("comments.moderate", json!({"id": id, "status": "published"})).await;
        assert!(missing, "`version` is required");
        let (bad, _) =
            call("comments.moderate", json!({"id": id, "status": "gone", "version": version}))
                .await;
        assert!(bad, "unknown status");
        let (failed, approved) =
            call("comments.moderate", json!({"id": id, "status": "published", "version": version}))
                .await;
        assert!(!failed, "{approved}");
        assert_eq!(approved["data"]["status"], "published");
        let (stale, _) =
            call("comments.moderate", json!({"id": id, "status": "spam", "version": version}))
                .await;
        assert!(stale, "an old version is refused");
        let (failed, closed) = call(
            "comments.set_target_state",
            json!({"target_type": "content.entry", "target_id": "public-1", "state": "closed"}),
        )
        .await;
        assert!(!failed, "{closed}");
        assert_eq!(closed["data"]["state"], "closed");
        let (wrong, _) = call(
            "comments.set_target_state",
            json!({"target_type": "content.entry", "target_id": "public-1", "state": "gone"}),
        )
        .await;
        assert!(wrong);
        admin.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
