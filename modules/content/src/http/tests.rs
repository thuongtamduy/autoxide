//! Test qua HTTP (router thật của kernel) và MCP (client thật): endpoint công khai không lộ bản
//! nháp, cache (`Cache-Control`, `ETag`, 304), `If-Match` bắt buộc, link xem trước, tool MCP
//! với tham số `version`. Gate chạy trên cả SQLite và Postgres.

use std::sync::Arc;

use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::error::Error;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_media::api::Media;
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

use crate::testing::FakeMedia;

#[cfg(test)]
mod fake {
    //! Bộ xác thực giả (chỉ có trong test; `cargo xtask check` bỏ qua phần `#[cfg(test)]`).

    use autoxide_kernel::access::{Authenticator, Principal};
    use autoxide_kernel::error::Error;
    use uuid::Uuid;

    /// Token → permission: `admin` mọi quyền của module; `reader` chỉ đọc.
    pub(super) struct FakeAuthenticator;

    #[async_trait::async_trait]
    impl Authenticator for FakeAuthenticator {
        async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
            let permissions: &[&str] = match authorization {
                Some("Bearer admin") => &[
                    "content.read",
                    "content.write",
                    "content.publish",
                    "content.delete",
                    "content.manage",
                ],
                Some("Bearer reader") => &["content.read"],
                Some("Bearer agent") => {
                    let permissions = ["content.read", "content.write_own"];
                    return Ok(Principal::new(Uuid::from_u128(2), permissions.map(str::to_owned))
                        .via_api_key(Uuid::from_u128(3)));
                }
                _ => {
                    return Err(Error::unauthenticated(
                        "auth.invalid_token",
                        "The token is not valid.",
                    ));
                }
            };
            Ok(Principal::new(Uuid::from_u128(1), permissions.iter().map(|p| (*p).to_owned())))
        }
    }
}

use fake::FakeAuthenticator;

/// Module giả công bố thư viện media, thay cho module `media`.
struct TestMedia(Arc<FakeMedia>);

impl Module for TestMedia {
    fn name(&self) -> &'static str {
        "testmedia"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let media: Arc<dyn Media> = self.0.clone();
        context.provide::<dyn Media>(media)
    }
}

const SECRET: &str = "a-test-secret-that-is-long-enough-0123456789";

async fn app(test: &TestDatabase) -> Router {
    let source = ConfigSource::from_pairs([
        ("DATABASE_URL", test.url()),
        ("AUTOXIDE_CONTENT_PREVIEW_SECRET", SECRET),
    ]);
    let mut reader = ConfigReader::new(&source);
    let config = Config::read(&mut reader);
    let media = Arc::new(FakeMedia::default());
    let mut modules =
        Modules::new(vec![Box::new(TestMedia(media)), Box::new(crate::module())]).unwrap();
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
}

/// Gửi request; `who`: token (`None`: không đăng nhập); `headers`: header thêm vào.
async fn send(
    router: &Router,
    method: &str,
    uri: &str,
    who: Option<&str>,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Answer {
    let mut request = Request::builder().method(method).uri(format!("/api/v1/content{uri}"));
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

/// Vòng đời qua HTTP và mọi endpoint công khai: nháp không bao giờ lộ; `If-Match` bắt buộc;
/// bản dịch thiếu là 404; cache được và trả 304; sitemap, feed, menu, link xem trước.
#[tokio::test]
async fn vong_doi_va_endpoint_cong_khai() {
    TestDatabase::run(async |test| {
        let router = app(test).await;
        let created = send(&router, "POST", "/entries", Some("admin"), &[], Some(json!({"kind": "post", "title": "Đường đến Đà Lạt"}))).await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", created.json());
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&router, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();

        let change = json!({"excerpt": "Một chuyến đi"});
        let missing = send(&router, "PATCH", &format!("/entries/{id}"), Some("admin"), &[], Some(change.clone())).await;
        assert_eq!((missing.status, missing.json()["code"].as_str()), (StatusCode::PRECONDITION_REQUIRED, Some("precondition_required")));
        let changed = send(&router, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &etag)], Some(change.clone())).await;
        assert_eq!(changed.status, StatusCode::OK, "{}", changed.json());
        let stale = send(&router, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &etag)], Some(change)).await;
        assert_eq!(stale.status, StatusCode::PRECONDITION_FAILED);
        let etag = changed.header(header::ETAG).to_owned();

        // Nháp: không endpoint công khai nào thấy.
        let path = "/public/resolve?locale=en&path=/blog/duong-den-da-lat";
        assert_eq!(send(&router, "GET", path, None, &[], None).await.status, StatusCode::NOT_FOUND);
        assert_eq!(send(&router, "GET", "/public/posts?locale=en", None, &[], None).await.json()["data"], json!([]));
        assert_eq!(send(&router, "GET", "/public/sitemap", None, &[], None).await.json()["data"], json!([]));
        assert_eq!(send(&router, "GET", "/public/feed?locale=en", None, &[], None).await.json()["data"], json!([]));

        let reader_publish = send(&router, "POST", &format!("/entries/{id}/publish"), Some("reader"), &[("if-match", &etag)], None).await;
        assert_eq!(reader_publish.status, StatusCode::FORBIDDEN);
        let published = send(&router, "POST", &format!("/entries/{id}/publish"), Some("admin"), &[("if-match", &etag)], None).await;
        assert_eq!(published.status, StatusCode::OK, "{}", published.json());

        let found = send(&router, "GET", path, None, &[], None).await;
        assert_eq!(found.status, StatusCode::OK, "{}", found.json());
        assert_eq!((found.json()["data"]["type"].as_str(), found.json()["data"]["entry"]["excerpt"].as_str()), (Some("entry"), Some("Một chuyến đi")));
        assert_eq!(found.header(header::CACHE_CONTROL), "public, no-cache");
        let public_etag = found.header(header::ETAG).to_owned();
        let again = send(&router, "GET", path, None, &[("if-none-match", &public_etag)], None).await;
        assert_eq!(again.status, StatusCode::NOT_MODIFIED);
        let vi = send(&router, "GET", "/public/resolve?locale=vi&path=/blog/duong-den-da-lat", None, &[], None).await;
        assert_eq!((vi.status, vi.json()["code"].as_str()), (StatusCode::NOT_FOUND, Some("content.not_found")), "no fallback");
        let unsupported = send(&router, "GET", "/public/resolve?locale=fr&path=/x", None, &[], None).await;
        assert_eq!(unsupported.status, StatusCode::UNPROCESSABLE_ENTITY);

        let posts = send(&router, "GET", "/public/posts?locale=en&q=da%20lat", None, &[], None).await.json();
        assert_eq!(posts["data"][0]["path"], "/blog/duong-den-da-lat");
        let sitemap = send(&router, "GET", "/public/sitemap?locale=en", None, &[], None).await.json();
        assert_eq!((sitemap["data"][0]["path"].as_str(), sitemap["data"][0]["kind"].as_str()), (Some("/blog/duong-den-da-lat"), Some("post")));
        assert_eq!(send(&router, "GET", "/public/feed?locale=en&limit=5", None, &[], None).await.json()["data"].as_array().unwrap().len(), 1);
        assert_eq!(send(&router, "GET", "/public/feed?locale=en&limit=0", None, &[], None).await.status, StatusCode::UNPROCESSABLE_ENTITY);
        let author = send(&router, "GET", "/public/authors/nobody", None, &[], None).await;
        assert_eq!(author.json()["code"], "content.author_not_found");

        let menu = json!({"items": [{"label": "Blog", "entry_id": id}]});
        let put = send(&router, "PUT", "/menus/main?locale=en", Some("admin"), &[("if-match", "*")], Some(menu)).await;
        assert_eq!(put.status, StatusCode::OK, "{}", put.json());
        let public_menu = send(&router, "GET", "/public/menus/main?locale=vi", None, &[], None).await.json();
        assert_eq!((public_menu["data"]["locale"].as_str(), public_menu["data"]["items"][0]["href"].as_str()), (Some("en"), Some("/blog/duong-den-da-lat")));

        // Link xem trước: thấy bản nháp mới nhất, không được cache.
        let draft = send(&router, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let draft_etag = draft.header(header::ETAG).to_owned();
        let edit = json!({"title": "Bản nháp mới"});
        send(&router, "PATCH", &format!("/entries/{id}"), Some("admin"), &[("if-match", &draft_etag)], Some(edit)).await;
        let denied = send(&router, "POST", &format!("/entries/{id}/preview-token"), Some("reader"), &[], None).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN);
        let token = send(&router, "POST", &format!("/entries/{id}/preview-token"), Some("admin"), &[], None).await.json();
        let token = token["data"]["token"].as_str().unwrap().to_owned();
        let preview = send(&router, "GET", &format!("/public/preview?token={token}"), None, &[], None).await;
        assert_eq!((preview.status, preview.json()["data"]["title"].as_str()), (StatusCode::OK, Some("Bản nháp mới")));
        assert_eq!(preview.header(header::CACHE_CONTROL), "no-store");
        assert_eq!(send(&router, "GET", path, None, &[], None).await.json()["data"]["entry"]["title"], "Đường đến Đà Lạt", "readers still see the published version");
        let bad = send(&router, "GET", "/public/preview?token=abc.def", None, &[], None).await;
        assert_eq!((bad.status, bad.json()["code"].as_str()), (StatusCode::NOT_FOUND, Some("content.preview_token_invalid")));
    })
    .await
    .unwrap();
}

/// Tool MCP qua client thật: người chỉ đọc chỉ thấy tool đọc; tạo nháp, sửa với `version`
/// (cũ thì bị từ chối), đăng, `resolve`, revision, khôi phục, gỡ.
#[tokio::test]
async fn tool_mcp_voi_version() {
    TestDatabase::run(async |test| {
        let router = app(test).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await });
        autoxide_kernel::tls::install_crypto_provider();
        let client = async |who: &str| {
            let config = StreamableHttpClientTransportConfig::with_uri(url.clone()).auth_header(who.to_owned());
            ().serve(StreamableHttpClientTransport::from_config(config)).await.unwrap()
        };
        let reader = client("reader").await;
        let mut names: Vec<String> = reader.list_all_tools().await.unwrap().into_iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["content.get", "content.list", "content.list_revisions", "content.resolve"]);
        reader.cancel().await.unwrap();

        // Agent chỉ có `content.write_own`: thấy tool viết, không thấy tool đăng.
        let agent = client("agent").await;
        let mut names: Vec<String> = agent.list_all_tools().await.unwrap().into_iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        assert_eq!(names, ["content.create_draft", "content.get", "content.list", "content.list_revisions", "content.request_review", "content.resolve", "content.restore_revision", "content.update"]);
        agent.cancel().await.unwrap();

        let admin = client("admin").await;
        let tools = admin.list_all_tools().await.unwrap();
        assert_eq!(tools.len(), 10);
        let publish = tools.iter().find(|tool| tool.name == "content.publish").unwrap();
        assert!(publish.description.as_deref().unwrap_or_default().contains("Requires permission `content.publish` or `content.publish_own`."));
        for tool in &tools {
            assert!(!serde_json::to_string(&tool.input_schema).unwrap().contains("$ref"), "{} has a $ref", tool.name);
        }
        let call = async |name: &str, arguments: Value| {
            let request = CallToolRequestParams::new(name.to_owned()).with_arguments(arguments.as_object().cloned().unwrap());
            let result = admin.call_tool(request).await.unwrap();
            (result.is_error == Some(true), result.structured_content.unwrap_or(Value::Null))
        };
        let body = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Xin chào"}]}]});
        let (failed, created) = call("content.create_draft", json!({"kind": "post", "title": "Hello MCP", "body": body})).await;
        assert!(!failed, "{created}");
        let id = created["data"]["id"].clone();
        let version = created["data"]["version"].as_str().unwrap().to_owned();
        assert_eq!(created["data"]["word_count"], 2);

        let (failed, updated) = call("content.update", json!({"id": id, "version": version, "excerpt": "Short", "save_revision": true})).await;
        assert!(!failed, "{updated}");
        let (stale, _) = call("content.update", json!({"id": id, "version": version, "excerpt": "Lost"})).await;
        assert!(stale, "an old version is refused");
        let (missing, _) = call("content.publish", json!({"id": id})).await;
        assert!(missing, "`version` is required");
        let version = updated["data"]["version"].as_str().unwrap().to_owned();
        let (failed, published) = call("content.publish", json!({"id": id, "version": version})).await;
        assert!(!failed, "{published}");
        assert_eq!(published["data"]["status"], "published");

        let (_, resolved) = call("content.resolve", json!({"locale": "en", "path": "/blog/hello-mcp"})).await;
        assert_eq!((resolved["data"]["type"].as_str(), resolved["data"]["entry"]["excerpt"].as_str()), (Some("entry"), Some("Short")));
        let (_, revisions) = call("content.list_revisions", json!({"id": id})).await;
        let revisions = revisions["data"].as_array().unwrap().clone();
        let initial = revisions.iter().find(|revision| revision["reason"] == "initial").unwrap();
        let (_, current) = call("content.get", json!({"id": id})).await;
        let version = current["data"]["version"].as_str().unwrap().to_owned();
        let (failed, restored) = call("content.restore_revision", json!({"id": id, "revision_id": initial["id"], "version": version})).await;
        assert!(!failed, "{restored}");
        assert_eq!((restored["data"]["excerpt"].as_str(), restored["data"]["has_unpublished_changes"].as_bool()), (Some(""), Some(true)));
        let version = restored["data"]["version"].as_str().unwrap().to_owned();
        let (failed, down) = call("content.unpublish", json!({"id": id, "version": version})).await;
        assert!(!failed, "{down}");
        let (gone, _) = call("content.resolve", json!({"locale": "en", "path": "/blog/hello-mcp"})).await;
        assert!(gone, "404 once taken down");
        let (_, listed) = call("content.list", json!({"status": "draft", "q": "xin chao"})).await;
        assert_eq!(listed["data"][0]["id"], id);
        admin.cancel().await.unwrap();
    })
    .await
    .unwrap();
}

/// ETag của `resolve` đổi khi thứ gộp vào câu trả lời đổi (ở đây: gắn thẻ), dù bản công khai
/// không đổi — website không được nhận 304 với nội dung cũ.
#[tokio::test]
async fn etag_resolve_theo_ca_cau_tra_loi() {
    TestDatabase::run(async |test| {
        let router = app(test).await;
        let created = send(
            &router,
            "POST",
            "/entries",
            Some("admin"),
            &[],
            Some(json!({"kind": "post", "title": "Tagged"})),
        )
        .await;
        let id = created.json()["data"]["id"].as_str().unwrap().to_owned();
        let read = send(&router, "GET", &format!("/entries/{id}"), Some("admin"), &[], None).await;
        let etag = read.header(header::ETAG).to_owned();
        let published = send(
            &router,
            "POST",
            &format!("/entries/{id}/publish"),
            Some("admin"),
            &[("if-match", &etag)],
            None,
        )
        .await;
        assert_eq!(published.status, StatusCode::OK, "{}", published.json());
        let path = "/public/resolve?locale=en&path=/blog/tagged";
        let first = send(&router, "GET", path, None, &[], None).await;
        let first_etag = first.header(header::ETAG).to_owned();
        assert!(!first_etag.is_empty());

        let tag = send(
            &router,
            "POST",
            "/terms",
            Some("admin"),
            &[],
            Some(json!({"taxonomy": "tag", "locale": "en", "name": "Food"})),
        )
        .await;
        assert_eq!(tag.status, StatusCode::CREATED, "{}", tag.json());
        let tag_id = tag.json()["data"]["id"].as_str().unwrap().to_owned();
        let linked = send(
            &router,
            "PUT",
            &format!("/entries/{id}/terms"),
            Some("admin"),
            &[],
            Some(json!({"term_ids": [tag_id]})),
        )
        .await;
        assert_eq!(linked.status, StatusCode::OK, "{}", linked.json());

        let again = send(&router, "GET", path, None, &[("if-none-match", &first_etag)], None).await;
        assert_eq!(again.status, StatusCode::OK, "the tag changed the answer");
        assert_ne!(again.header(header::ETAG), first_etag);
        let same = send(
            &router,
            "GET",
            path,
            None,
            &[("if-none-match", again.header(header::ETAG))],
            None,
        )
        .await;
        assert_eq!(same.status, StatusCode::NOT_MODIFIED);
    })
    .await
    .unwrap();
}
