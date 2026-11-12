//! Test qua HTTP (router thật của kernel, cùng module `antispam` và job worker thật) và MCP
//! (client thật): đăng ký công khai trả lời giống hệt, xác nhận bằng `POST`, gửi chiến dịch cần
//! `If-Match` và `newsletter.send`, huỷ một chạm bằng thân form-urlencoded, trang lựa chọn, xuất /
//! xoá theo email, tool MCP. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
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

use crate::testing::FakeMailer;

#[cfg(test)]
mod fake {
    //! Bộ xác thực giả (chỉ có trong test; `cargo xtask check` bỏ qua phần `#[cfg(test)]`).

    use autoxide_kernel::access::{Authenticator, Principal};
    use autoxide_kernel::error::Error;
    use uuid::Uuid;

    /// Token → permission: `admin` mọi quyền; `reader` chỉ đọc; `writer` đọc và soạn; `website`
    /// chỉ `newsletter.subscribe` (máy chủ của website gọi đăng ký bằng API key); `member` không
    /// quyền nào (tài khoản tự đăng ký).
    pub(super) struct FakeAuthenticator;

    #[async_trait::async_trait]
    impl Authenticator for FakeAuthenticator {
        async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
            let (id, permissions): (u128, &[&str]) = match authorization {
                Some("Bearer admin") => (
                    1,
                    &[
                        "newsletter.read",
                        "newsletter.manage",
                        "newsletter.send",
                        "newsletter.export",
                    ],
                ),
                Some("Bearer reader") => (2, &["newsletter.read"]),
                Some("Bearer writer") => (3, &["newsletter.read", "newsletter.manage"]),
                Some("Bearer website") => (4, &["newsletter.subscribe"]),
                Some("Bearer member") => (5, &[]),
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

struct App {
    router: Router,
    mailer: Arc<FakeMailer>,
    modules: Modules,
    database: Database,
}

impl App {
    /// Chạy các job tới hạn tới khi hết (như worker của chương trình).
    async fn work(&self) {
        let worker = self.modules.worker(&self.database, 4).unwrap();
        for _ in 0..20 {
            if worker.run_once().await.unwrap() == 0 {
                break;
            }
        }
    }
}

async fn app(test: &TestDatabase) -> App {
    let source = ConfigSource::from_pairs([
        ("DATABASE_URL", test.url()),
        // Không chờ 3 giây giữa lấy token và gửi.
        ("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0"),
        ("AUTOXIDE_HTTP_PUBLIC_URL", "https://api.example.com"),
        ("AUTOXIDE_NEWSLETTER_CONFIRM_URL", "https://www.example.com/confirm?token={token}"),
        ("AUTOXIDE_NEWSLETTER_PREFERENCES_URL", "https://www.example.com/prefs?token={token}"),
        ("AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS", "Example Ltd, Hanoi"),
    ]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let mailer = Arc::new(FakeMailer::default());
    let mut modules =
        Modules::new(vec![Box::new(TestMail(mailer.clone())), Box::new(crate::module())]).unwrap();
    let configured = modules.configure(&mut reader);
    let (config, _) = reader.finish(config.and_then(|config| configured.map(|_| config))).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
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
    App { router, mailer, modules, database }
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
    let content_type = body.as_ref().map(|_| "application/json");
    let body = body.map(|body| body.to_string());
    raw(router, method, uri, who, headers, content_type, body).await
}

async fn raw(
    router: &Router,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    content_type: Option<&str>,
    body: Option<String>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(format!("/api/v1{uri}"));
    if let Some(who) = who {
        request = request.header("authorization", format!("Bearer {who}"));
    }
    if let Some(content_type) = content_type {
        request = request.header("content-type", content_type);
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let body = body.map_or_else(Body::empty, Body::from);
    let response = router.clone().oneshot(request.body(body).unwrap()).await;
    let (parts, body) = response.unwrap().into_parts();
    let body = axum::body::to_bytes(body, usize::MAX).await.unwrap().to_vec();
    Answer { status: parts.status, headers: parts.headers, body }
}

async fn form_token(router: &Router) -> String {
    let issued =
        send(router, "GET", "/antispam/token?scope=newsletter:subscribe", None, &[], None).await;
    assert_eq!(issued.status, StatusCode::OK, "{}", issued.json());
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
        let router = &app.router;
        let created = send(router, "POST", "/newsletter/lists", Some("admin"), &[], Some(list_body())).await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.json());
        let list_id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created.header(header::LOCATION), format!("/api/v1/newsletter/lists/{list_id}"));
        let taken = send(router, "POST", "/newsletter/lists", Some("admin"), &[], Some(list_body())).await;
        assert_eq!((taken.status, taken.code().as_deref()), (StatusCode::CONFLICT, Some("newsletter.list_key_taken")));
        assert_eq!(send(router, "POST", "/newsletter/lists", Some("reader"), &[], Some(list_body())).await.status, StatusCode::FORBIDDEN);
        let public = send(router, "GET", "/newsletter/public/lists?locale=vi", None, &[], None).await;
        assert_eq!(public.json()["data"]["token_scope"], "newsletter:subscribe");
        assert_eq!(public.json()["data"]["lists"][0]["name"], "Tin hằng tuần");

        // Đăng ký: mới, lặp lại, dính bẫy — cùng mã, cùng thân.
        let body = |token: String, email: &str| json!({"email": email, "lists": ["weekly"], "locale": "vi", "consent_text": "Gửi tôi bản tin.", "form_token": token});
        let first = send(router, "POST", "/newsletter/public/subscribe", None, &[], Some(body(form_token(router).await, "ada@example.com"))).await;
        assert_eq!(first.status, StatusCode::ACCEPTED, "{}", first.json());
        let again = send(router, "POST", "/newsletter/public/subscribe", None, &[], Some(body(form_token(router).await, "ada@example.com"))).await;
        let mut trapped = body(form_token(router).await, "bot@example.com");
        trapped["trap"] = json!("x");
        let trapped = send(router, "POST", "/newsletter/public/subscribe", None, &[], Some(trapped)).await;
        for other in [&again, &trapped] {
            assert_eq!((other.status, &other.body), (first.status, &first.body));
        }
        let missing = send(router, "POST", "/newsletter/public/subscribe", None, &[], Some(json!({"email": "c@example.com", "lists": ["weekly"]}))).await;
        assert_eq!((missing.status, missing.code().as_deref()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));
        let unknown = send(router, "POST", "/newsletter/public/subscribe", None, &[], Some(json!({"email": "c@example.com", "lists": ["nope"], "form_token": form_token(router).await}))).await;
        assert_eq!((unknown.status, unknown.json()["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("lists[0]")));
        // Website gọi bằng API key: không cần token.
        let by_website = send(router, "POST", "/newsletter/public/subscribe", Some("website"), &[], Some(json!({"email": "web@example.com", "lists": ["weekly"]}))).await;
        assert_eq!(by_website.status, StatusCode::ACCEPTED);
        // Đăng nhập mà không có `newsletter.subscribe`: như người lạ, thiếu token thì bị từ chối.
        let by_member = send(router, "POST", "/newsletter/public/subscribe", Some("member"), &[], Some(json!({"email": "member@example.com", "lists": ["weekly"]}))).await;
        assert_eq!((by_member.status, by_member.code().as_deref()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));

        // Worker lưu các yêu cầu và gửi mail xác nhận (D-154).
        assert!(app.mailer.to("ada@example.com").is_empty());
        app.work().await;

        // Xác nhận: trang của website gọi `POST`.
        let mails = app.mailer.to("ada@example.com");
        assert_eq!(mails.len(), 2);
        let confirm = json!({"token": token_in(&mails[1].text)});
        let confirmed = send(router, "POST", "/newsletter/public/confirm", None, &[], Some(confirm.clone())).await;
        assert_eq!(confirmed.status, StatusCode::OK, "{}", confirmed.json());
        assert_eq!(confirmed.json()["data"]["lists"][0]["key"], "weekly");
        let reused = send(router, "POST", "/newsletter/public/confirm", None, &[], Some(confirm)).await;
        assert_eq!((reused.status, reused.code().as_deref()), (StatusCode::BAD_REQUEST, Some("newsletter.confirmation_invalid")));

        // Chiến dịch: người soạn tạo được nhưng không gửi được; gửi cần `If-Match`.
        let draft = json!({"list_id": list_id, "subject": "Hello", "body_text": "News of the week."});
        let campaign = send(router, "POST", "/newsletter/campaigns", Some("writer"), &[], Some(draft.clone())).await;
        assert_eq!(campaign.status, StatusCode::CREATED, "{}", campaign.json());
        assert_eq!(send(router, "POST", "/newsletter/campaigns", Some("reader"), &[], Some(draft)).await.status, StatusCode::FORBIDDEN);
        let id = campaign.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(router, "GET", &format!("/newsletter/campaigns/{id}"), Some("reader"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let path = format!("/newsletter/campaigns/{id}/send");
        assert_eq!(send(router, "POST", &path, Some("admin"), &[], None).await.status, StatusCode::PRECONDITION_REQUIRED);
        assert_eq!(send(router, "POST", &path, Some("writer"), &[("if-match", &etag)], None).await.status, StatusCode::FORBIDDEN);
        let edit = send(router, "PATCH", &format!("/newsletter/campaigns/{id}"), Some("writer"), &[("if-match", &etag)], Some(json!({"subject": "Hello again"}))).await;
        assert_eq!(edit.status, StatusCode::OK, "{}", edit.json());
        assert_eq!(send(router, "POST", &path, Some("admin"), &[("if-match", &etag)], None).await.status, StatusCode::PRECONDITION_FAILED);
        let fresh = edit.header(header::ETAG).to_owned();
        let started = send(router, "POST", &path, Some("admin"), &[("if-match", &fresh)], None).await;
        assert_eq!((started.status, started.json()["data"]["status"].as_str()), (StatusCode::OK, Some("sending")), "{}", started.json());
        app.work().await;
        let mail = app.mailer.to("ada@example.com").pop().unwrap();
        assert_eq!(mail.subject, "Hello again");
        let header = mail.headers.iter().find(|(name, _)| name == "List-Unsubscribe").unwrap().1.clone();
        let progress = send(router, "GET", &format!("/newsletter/campaigns/{id}"), Some("reader"), &[], None).await;
        assert_eq!(progress.json()["data"]["progress"]["sent"], 1, "{}", progress.json());

        // Trang lựa chọn, rồi huỷ một chạm như ứng dụng mail (thân form-urlencoded).
        let token = token_in(&header);
        let prefs = send(router, "GET", &format!("/newsletter/public/preferences?token={token}"), None, &[], None).await;
        assert_eq!((prefs.status, prefs.json()["data"]["lists"][0]["subscribed"].as_bool()), (StatusCode::OK, Some(true)), "{}", prefs.json());
        let one_click = raw(router, "POST", &format!("/newsletter/public/unsubscribe?token={token}"), None, &[], Some("application/x-www-form-urlencoded"), Some("List-Unsubscribe=One-Click".to_owned())).await;
        assert_eq!(one_click.status, StatusCode::OK, "{}", String::from_utf8_lossy(&one_click.body));
        let unknown = raw(router, "POST", "/newsletter/public/unsubscribe?token=nope", None, &[], Some("application/x-www-form-urlencoded"), Some("List-Unsubscribe=One-Click".to_owned())).await;
        assert_eq!((unknown.status, &unknown.body), (one_click.status, &one_click.body));
        let left = send(router, "GET", &format!("/newsletter/subscribers?list_id={list_id}&status=unsubscribed"), Some("reader"), &[], None).await;
        assert_eq!(left.json()["data"][0]["email"], "ada@example.com", "{}", left.json());
        let back = send(router, "PUT", &format!("/newsletter/public/preferences?token={token}"), None, &[], Some(json!({"lists": [{"key": "weekly", "subscribed": true}]}))).await;
        assert_eq!(back.json()["data"]["lists"][0]["status"], "confirmed", "{}", back.json());
        assert_eq!(send(router, "GET", "/newsletter/public/preferences?token=nope", None, &[], None).await.status, StatusCode::NOT_FOUND);

        // Danh sách: sửa cần `If-Match`.
        assert_eq!(send(router, "PATCH", &format!("/newsletter/lists/{list_id}"), Some("admin"), &[], Some(json!({"public": false}))).await.status, StatusCode::PRECONDITION_REQUIRED);

        // Bounce, danh sách chặn, xuất / xoá theo email.
        let bounce = send(router, "POST", "/newsletter/bounces", Some("admin"), &[], Some(json!({"email": "web@example.com", "type": "complaint"}))).await;
        assert_eq!(bounce.json()["data"]["suppressed"], true, "{}", bounce.json());
        let suppressions = send(router, "GET", "/newsletter/suppressions", Some("reader"), &[], None).await;
        assert_eq!(suppressions.json()["data"][0]["reason"], "complaint");
        let data = send(router, "GET", "/newsletter/data?email=ADA@example.com", Some("admin"), &[], None).await;
        assert_eq!(data.status, StatusCode::OK, "{}", data.json());
        assert_eq!(data.json()["data"]["deliveries"].as_array().unwrap().len(), 1);
        assert_eq!(send(router, "GET", "/newsletter/data?email=ada@example.com", Some("reader"), &[], None).await.status, StatusCode::FORBIDDEN);
        assert_eq!(send(router, "GET", "/newsletter/data", Some("admin"), &[], None).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        let erased = send(router, "DELETE", "/newsletter/data?email=ada@example.com", Some("admin"), &[], None).await;
        assert_eq!(erased.json()["data"]["deleted"], true);
        let after = send(router, "GET", "/newsletter/data?email=ada@example.com", Some("admin"), &[], None).await;
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
            send(&app.router, "POST", "/newsletter/lists", Some("admin"), &[], Some(list_body()))
                .await;
        let list_id = created.json()["data"]["id"].clone();
        let router = app.router.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        autoxide_kernel::tls::install_crypto_provider();
        let client = async |who: &str| {
            let config = StreamableHttpClientTransportConfig::with_uri(url.clone())
                .auth_header(who.to_owned());
            ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap()
        };
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
            let request = CallToolRequestParams::new(name.to_owned())
                .with_arguments(arguments.as_object().cloned().unwrap());
            let result = admin.call_tool(request).await.unwrap();
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
