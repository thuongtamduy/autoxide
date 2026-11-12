//! Test qua HTTP (router thật của kernel, cùng module `antispam` thật) và MCP (client thật):
//! endpoint công khai không lộ phần riêng tư, câu trả lời giống hệt cho bài nhận / dính bẫy /
//! quá nhiều lần, gửi lại cùng token, `If-Match` bắt buộc, CSV, xuất / xoá theo email, giới hạn
//! thân request, tool MCP. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
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

    /// Token → permission: `admin` mọi quyền của module; `reader` chỉ đọc; `website` không
    /// quyền nào (máy chủ của website gửi form bằng API key).
    pub(super) struct FakeAuthenticator;

    #[async_trait::async_trait]
    impl Authenticator for FakeAuthenticator {
        async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
            let (id, permissions): (u128, &[&str]) = match authorization {
                Some("Bearer admin") => {
                    (1, &["forms.read", "forms.manage", "forms.review", "forms.export"])
                }
                Some("Bearer reader") => (2, &["forms.read"]),
                Some("Bearer website") => (3, &[]),
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

async fn app(test: &TestDatabase) -> Router {
    let source = ConfigSource::from_pairs([
        ("DATABASE_URL", test.url()),
        // Không chờ 3 giây giữa lấy token và gửi.
        ("AUTOXIDE_ANTISPAM_MIN_FILL_SECONDS", "0"),
        ("AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES", "2048"),
    ]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let mailer = Arc::new(FakeMailer::default());
    let mut modules =
        Modules::new(vec![Box::new(TestMail(mailer)), Box::new(crate::module())]).unwrap();
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
    autoxide_kernel::http::build(
        &config.http,
        utoipa_axum::router::OpenApiRouter::new(),
        modules.routes(),
        extras,
    )
    .router
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

fn definition() -> Value {
    json!({
        "key": "contact",
        "name": "Contact us",
        "fields": [
            {"key": "name", "type": "text", "label": {"en": "Name", "vi": "Tên"}, "required": true},
            {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
            {"key": "message", "type": "textarea", "label": {"en": "Message"}},
            {"key": "topic", "type": "select", "label": {"en": "Topic"}, "options": [{"value": "sales", "label": {"en": "Sales", "vi": "Bán hàng"}}]}
        ],
        "allowed_origins": ["https://example.com"],
        "notify_emails": ["owner@example.com"],
        "success_message": {"en": "Thanks!", "vi": "Cảm ơn!"},
        "email_limit_per_hour": 2
    })
}

async fn form_token(router: &Router) -> String {
    let issued = send(router, "GET", "/antispam/token?scope=forms:contact", None, &[], None).await;
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

/// Vòng đời qua HTTP: tạo form, định nghĩa công khai không lộ phần riêng tư, gửi bài, câu trả
/// lời giống hệt cho bài dính bẫy và bài vượt giới hạn, gửi lại cùng token, lỗi kiểm dữ liệu,
/// `Origin`, form đóng, `If-Match`, hộp thư, CSV, xuất / xoá theo email, thân quá lớn.
#[tokio::test]
async fn vong_doi_qua_http() {
    TestDatabase::run(async |test| {
        let router = app(test).await;
        let created = send(&router, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(created.header(header::LOCATION), format!("/api/v1/forms/{id}"));
        assert_eq!(created.json()["data"]["token_scope"], "forms:contact");
        let taken = send(&router, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        assert_eq!((taken.status, taken.json()["code"].as_str()), (StatusCode::CONFLICT, Some("forms.key_taken")));
        let denied = send(&router, "POST", "/forms", Some("reader"), &[], Some(definition())).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN);

        // Định nghĩa công khai: không lộ email nhận thông báo, giới hạn, origin.
        let public = send(&router, "GET", "/forms/public/contact?locale=vi", None, &[], None).await;
        assert_eq!(public.status, StatusCode::OK, "{}", public.json());
        let text = public.text();
        for private in ["owner@example.com", "notify", "allowed_origins", "email_limit", "retention"] {
            assert!(!text.contains(private), "public form shows `{private}`: {text}");
        }
        assert_eq!(public.json()["data"]["fields"][0]["label"], "Tên");
        assert_eq!(public.json()["data"]["fields"][3]["options"][0]["label"], "Bán hàng");
        assert_eq!(public.json()["data"]["token_scope"], "forms:contact");
        let etag = public.header(header::ETAG).to_owned();
        let cached = send(&router, "GET", "/forms/public/contact?locale=vi", None, &[("if-none-match", &etag)], None).await;
        assert_eq!(cached.status, StatusCode::NOT_MODIFIED);
        assert_eq!(send(&router, "GET", "/forms/public/nope", None, &[], None).await.status, StatusCode::NOT_FOUND);
        let french = send(&router, "GET", "/forms/public/contact?locale=fr", None, &[], None).await;
        assert_eq!((french.status, french.json()["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("locale")));

        // Bài thật.
        let origin = [("origin", "https://example.com")];
        let body = |token: &str, email: &str| json!({
            "data": {"name": "Ada", "email": email, "message": "Hello", "topic": "sales"},
            "locale": "vi", "form_token": token
        });
        let token = form_token(&router).await;
        let accepted = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&token, "ada@example.com"))).await;
        assert_eq!(accepted.status, StatusCode::CREATED, "{}", accepted.json());
        assert_eq!(accepted.json()["data"]["message"], "Cảm ơn!");
        let submission = accepted.json()["data"]["id"].as_str().unwrap().to_owned();
        // Gửi lại cùng token: cùng câu trả lời, không thêm bài.
        let again = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&token, "ada@example.com"))).await;
        assert_eq!((again.status, again.json()["data"]["id"].as_str()), (StatusCode::CREATED, Some(submission.as_str())));

        // Dính bẫy: cùng mã, cùng hình dạng, cùng lời nhắn.
        let mut trapped = body(&form_token(&router).await, "bot@example.com");
        trapped["trap"] = json!("http://spam.example");
        let trapped = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(trapped)).await;
        assert_eq!((trapped.status, shape(&trapped), &trapped.json()["data"]["message"]), (accepted.status, shape(&accepted), &accepted.json()["data"]["message"]));
        assert_eq!(trapped.header(header::LOCATION).len(), accepted.header(header::LOCATION).len());
        // Quá nhiều lần với một email (giới hạn 2/giờ, đã đếm 2): câu trả lời y hệt, không lưu.
        let limited = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&form_token(&router).await, "ada@example.com"))).await;
        assert_eq!((limited.status, shape(&limited)), (accepted.status, shape(&accepted)));
        assert_ne!(limited.json()["data"]["id"], accepted.json()["data"]["id"]);

        // Lỗi thật thì rõ ràng: dữ liệu sai (mọi lỗi một lần), token thiếu, `Origin` lạ.
        let wrong = json!({"data": {"email": "nope", "topic": "other", "extra": 1}, "form_token": form_token(&router).await});
        let invalid = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(wrong)).await;
        assert_eq!(invalid.status, StatusCode::UNPROCESSABLE_ENTITY);
        let mut fields: Vec<String> = invalid.json()["errors"].as_array().unwrap().iter().map(|e| e["field"].as_str().unwrap().to_owned()).collect();
        fields.sort();
        assert_eq!(fields, ["data.email", "data.extra", "data.name", "data.topic"]);
        let no_token = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(json!({"data": {"name": "A", "email": "a@example.com"}}))).await;
        assert_eq!((no_token.status, no_token.json()["code"].as_str()), (StatusCode::BAD_REQUEST, Some("antispam.token_missing")));
        let foreign = send(&router, "POST", "/forms/public/contact/submissions", None, &[("origin", "https://evil.example")], Some(body(&form_token(&router).await, "c@example.com"))).await;
        assert_eq!((foreign.status, foreign.json()["code"].as_str()), (StatusCode::FORBIDDEN, Some("forms.origin_not_allowed")));
        // Thân quá lớn (giới hạn 2048 byte trong test).
        let huge = json!({"data": {"name": "A", "email": "a@example.com", "message": "x".repeat(3000)}, "form_token": "t"});
        let too_large = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(huge)).await;
        assert_eq!(too_large.status, StatusCode::PAYLOAD_TOO_LARGE);
        // Website gọi bằng API key: không cần token.
        let signed = json!({"data": {"name": "Web", "email": "web@example.com"}});
        let by_website = send(&router, "POST", "/forms/public/contact/submissions", Some("website"), &[], Some(signed)).await;
        assert_eq!(by_website.status, StatusCode::CREATED, "{}", by_website.json());

        // Hộp thư: hai bài (Ada, Web); đổi trạng thái cần `If-Match`.
        let inbox = send(&router, "GET", &format!("/forms/{id}/submissions"), Some("reader"), &[], None).await;
        assert_eq!(inbox.json()["data"].as_array().unwrap().len(), 2, "{}", inbox.json());
        let one = send(&router, "GET", &format!("/forms/submissions/{submission}"), Some("reader"), &[], None).await;
        assert_eq!(one.json()["data"]["answers"][0]["label"]["vi"], "Tên");
        assert_eq!(one.json()["data"]["email"], "ada@example.com");
        let version = one.header(header::ETAG).to_owned();
        let mark = json!({"status": "read"});
        let missing = send(&router, "PATCH", &format!("/forms/submissions/{submission}"), Some("admin"), &[], Some(mark.clone())).await;
        assert_eq!(missing.status, StatusCode::PRECONDITION_REQUIRED);
        let by_reader = send(&router, "PATCH", &format!("/forms/submissions/{submission}"), Some("reader"), &[("if-match", &version)], Some(mark.clone())).await;
        assert_eq!(by_reader.status, StatusCode::FORBIDDEN);
        let marked = send(&router, "PATCH", &format!("/forms/submissions/{submission}"), Some("admin"), &[("if-match", &version)], Some(mark.clone())).await;
        assert_eq!((marked.status, marked.json()["data"]["status"].as_str()), (StatusCode::OK, Some("read")));
        let stale = send(&router, "PATCH", &format!("/forms/submissions/{submission}"), Some("admin"), &[("if-match", &version)], Some(json!({"status": "archived"}))).await;
        assert_eq!(stale.status, StatusCode::PRECONDITION_FAILED);

        // CSV.
        let csv = send(&router, "GET", &format!("/forms/{id}/submissions/export?format=csv"), Some("admin"), &[], None).await;
        assert_eq!(csv.status, StatusCode::OK);
        assert_eq!(csv.header(header::CONTENT_TYPE), "text/csv; charset=utf-8");
        assert!(csv.header(header::CONTENT_DISPOSITION).starts_with("attachment; filename=\"contact-submissions.csv\""));
        assert!(csv.text().contains("\"ada@example.com\""));
        assert_eq!(send(&router, "GET", &format!("/forms/{id}/submissions/export"), Some("reader"), &[], None).await.status, StatusCode::FORBIDDEN);
        assert_eq!(send(&router, "GET", &format!("/forms/{id}/submissions/export?format=xlsx"), Some("admin"), &[], None).await.status, StatusCode::UNPROCESSABLE_ENTITY);

        // Xuất / xoá theo email.
        let data = send(&router, "GET", "/forms/data?email=ADA@example.com", Some("admin"), &[], None).await;
        assert_eq!(data.status, StatusCode::OK, "{}", data.json());
        assert_eq!(data.json()["data"]["submissions"].as_array().unwrap().len(), 1);
        assert_eq!(send(&router, "GET", "/forms/data?email=ada@example.com", Some("reader"), &[], None).await.status, StatusCode::FORBIDDEN);
        assert_eq!(send(&router, "GET", "/forms/data", Some("admin"), &[], None).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        let erased = send(&router, "DELETE", "/forms/data?email=ada@example.com", Some("admin"), &[], None).await;
        assert_eq!(erased.json()["data"]["deleted"], 1);
        assert_eq!(send(&router, "GET", &format!("/forms/submissions/{submission}"), Some("admin"), &[], None).await.status, StatusCode::NOT_FOUND);

        // Sửa form: `If-Match` bắt buộc; đóng form thì gửi bị từ chối rõ ràng.
        let form = send(&router, "GET", &format!("/forms/{id}"), Some("admin"), &[], None).await;
        let form_etag = form.header(header::ETAG).to_owned();
        let close = json!({"status": "closed"});
        assert_eq!(send(&router, "PATCH", &format!("/forms/{id}"), Some("admin"), &[], Some(close.clone())).await.status, StatusCode::PRECONDITION_REQUIRED);
        let closed = send(&router, "PATCH", &format!("/forms/{id}"), Some("admin"), &[("if-match", &form_etag)], Some(close)).await;
        assert_eq!(closed.status, StatusCode::OK, "{}", closed.json());
        let refused = send(&router, "POST", "/forms/public/contact/submissions", None, &origin, Some(body(&form_token(&router).await, "z@example.com"))).await;
        assert_eq!((refused.status, refused.json()["code"].as_str()), (StatusCode::CONFLICT, Some("forms.closed")));
        assert_eq!(send(&router, "DELETE", &format!("/forms/{id}"), Some("admin"), &[("if-match", &form_etag)], None).await.status, StatusCode::PRECONDITION_FAILED);
        let new_etag = closed.header(header::ETAG).to_owned();
        assert_eq!(send(&router, "DELETE", &format!("/forms/{id}"), Some("admin"), &[("if-match", &new_etag)], None).await.status, StatusCode::NO_CONTENT);
    })
    .await
    .unwrap();
}

/// Tool MCP qua client thật: người chỉ đọc chỉ thấy tool đọc; không có tool gửi form; đổi
/// trạng thái cần `version` đúng.
#[tokio::test]
async fn tool_mcp() {
    TestDatabase::run(async |test| {
        let router = app(test).await;
        let created = send(&router, "POST", "/forms", Some("admin"), &[], Some(definition())).await;
        let form_id = created.json()["data"]["id"].clone();
        let token = form_token(&router).await;
        let submit =
            json!({"data": {"name": "Ada", "email": "ada@example.com"}, "form_token": token});
        let sent =
            send(&router, "POST", "/forms/public/contact/submissions", None, &[], Some(submit))
                .await;
        let submission = sent.json()["data"]["id"].clone();

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
        assert_eq!(
            names,
            ["forms.get", "forms.get_submission", "forms.list", "forms.list_submissions"]
        );
        reader.cancel().await.unwrap();

        let admin = client("admin").await;
        let tools = admin.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 5);
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
        let (failed, listed) = call("forms.list", json!({"status": "open"})).await;
        assert!(!failed, "{listed}");
        assert_eq!(listed["data"][0]["key"], "contact");
        let (_, form) = call("forms.get", json!({"id": form_id})).await;
        assert_eq!(form["data"]["fields"].as_array().unwrap().len(), 4);
        let (_, inbox) =
            call("forms.list_submissions", json!({"form_id": form_id, "q": "ada"})).await;
        assert_eq!(inbox["data"][0]["id"], submission);
        let (_, one) = call("forms.get_submission", json!({"id": submission})).await;
        let version = one["data"]["version"].as_str().unwrap().to_owned();
        assert_eq!(one["data"]["data"]["name"], "Ada");

        let (missing, _) =
            call("forms.set_submission_status", json!({"id": submission, "status": "archived"}))
                .await;
        assert!(missing, "`version` is required");
        let (bad, _) = call(
            "forms.set_submission_status",
            json!({"id": submission, "status": "gone", "version": version}),
        )
        .await;
        assert!(bad, "unknown status");
        let (failed, archived) = call(
            "forms.set_submission_status",
            json!({"id": submission, "status": "archived", "version": version}),
        )
        .await;
        assert!(!failed, "{archived}");
        assert_eq!(archived["data"]["status"], "archived");
        let (stale, _) = call(
            "forms.set_submission_status",
            json!({"id": submission, "status": "read", "version": version}),
        )
        .await;
        assert!(stale, "an old version is refused");
        admin.cancel().await.unwrap();
    })
    .await
    .unwrap();
}
