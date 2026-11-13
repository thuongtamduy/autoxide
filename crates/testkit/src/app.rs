//! [`TestApp`]: dựng module như binary dựng (cấu hình → migration → `start` → router của
//! kernel) trên một [`TestDatabase`], rồi gọi qua REST hoặc client MCP thật.
//!
//! Trước đây mỗi module tự viết lại khoảng 60–100 dòng dựng app, bộ xác thực giả và hàm gửi
//! request (D-164). Giờ chỉ cần:
//!
//! ```no_run
//! # async fn example(test: &autoxide_testkit::TestDatabase, module: impl autoxide_kernel::module::Module + 'static) {
//! use autoxide_testkit::TestApp;
//!
//! let app = TestApp::builder(test)
//!     .setting("AUTOXIDE_NOTES_MAX_TITLE", "80")
//!     .user("ada", 0xada, &["notes.read", "notes.write"])
//!     .module(module)
//!     .build()
//!     .await
//!     .unwrap();
//! let answer = app.post("/api/v1/notes").token("ada").json(serde_json::json!({})).send().await.unwrap();
//! assert_eq!(answer.status(), 422);
//! # }
//! ```
//!
//! Dựng giống `crates/app/src/runtime.rs`: cùng bộ xác thực (module công bố, hoặc bộ giả khi
//! test khai `user`), cùng idempotency, deprecation, giới hạn thân request, tool MCP, bản dịch.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Principal};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{self, Extras};
use autoxide_kernel::idempotency::IdempotencyStore;
use autoxide_kernel::module::{Module, Modules, StartContext};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header};
use rmcp::ServiceExt as _;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::Value;
use tower::ServiceExt as _;
use utoipa::openapi::OpenApi;
use uuid::Uuid;

use crate::TestDatabase;

/// Lỗi khi dựng hay gọi [`TestApp`]. Thông báo nói bước nào hỏng.
#[derive(Debug)]
pub struct TestAppError(String);

impl fmt::Display for TestAppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "test app: {}", self.0)
    }
}

impl std::error::Error for TestAppError {}

fn fail(step: &str, error: impl fmt::Display) -> TestAppError {
    TestAppError(format!("{step}: {error}"))
}

/// Ứng dụng dựng sẵn cho một bài test. Lấy qua [`TestApp::builder`].
pub struct TestApp {
    router: Router,
    openapi: OpenApi,
    // Giữ module sống cùng app: chúng sở hữu service của mình.
    modules: Modules,
    database: Database,
}

/// Cấu hình của [`TestApp`] trước khi dựng.
pub struct TestAppBuilder<'a> {
    test: &'a TestDatabase,
    settings: Vec<(String, String)>,
    modules: Vec<Box<dyn Module>>,
    provided: Vec<Provision>,
    users: HashMap<String, Principal>,
}

// `for<'c>`: hàm nhận `StartContext` với lifetime bất kỳ — lifetime thật chỉ biết lúc
// `start` chạy, không phải lúc test gọi `provide`.
type Provision = Box<dyn for<'c> FnOnce(&mut StartContext<'c>) -> Result<(), Error> + Send + Sync>;

impl TestApp {
    /// Bắt đầu dựng app trên database của bài test.
    pub fn builder(test: &TestDatabase) -> TestAppBuilder<'_> {
        TestAppBuilder {
            test,
            settings: Vec::new(),
            modules: Vec::new(),
            provided: Vec::new(),
            users: HashMap::new(),
        }
    }

    /// Router đầy đủ (mọi lớp HTTP của kernel), để tự gửi request khi helper không đủ.
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// Tài liệu OpenAPI của app.
    pub fn openapi(&self) -> &OpenApi {
        &self.openapi
    }

    /// Các module đã khởi động (lấy trait công bố, worker, dispatcher…).
    pub fn modules(&self) -> &Modules {
        &self.modules
    }

    /// Database của app (đọc kiểm kết quả, chạy job).
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// Trait một module đã công bố (`context.provide`), vd service của chính module.
    pub fn api<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.modules.api::<T>()
    }

    /// Bắt đầu một request. `uri` là đường dẫn đầy đủ, vd `/api/v1/notes/{id}`.
    pub fn request(&self, method: Method, uri: &str) -> TestRequest<'_> {
        TestRequest { app: self, builder: Request::builder().method(method).uri(uri), body: None }
    }

    pub fn get(&self, uri: &str) -> TestRequest<'_> {
        self.request(Method::GET, uri)
    }

    pub fn post(&self, uri: &str) -> TestRequest<'_> {
        self.request(Method::POST, uri)
    }

    pub fn put(&self, uri: &str) -> TestRequest<'_> {
        self.request(Method::PUT, uri)
    }

    pub fn patch(&self, uri: &str) -> TestRequest<'_> {
        self.request(Method::PATCH, uri)
    }

    pub fn delete(&self, uri: &str) -> TestRequest<'_> {
        self.request(Method::DELETE, uri)
    }

    /// Client MCP thật nối tới `/mcp` của app, xác thực bằng `token` (như [`TestRequest::token`]).
    ///
    /// Mở một cổng TCP cục bộ: transport Streamable HTTP của client cần máy chủ thật.
    pub async fn mcp(&self, token: &str) -> Result<RunningService<RoleClient, ()>, TestAppError> {
        let listener =
            tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| fail("mcp", e))?;
        let address = listener.local_addr().map_err(|e| fail("mcp", e))?;
        let router = self.router.clone();
        tokio::spawn(async move { axum::serve(listener, router).await });
        autoxide_kernel::tls::install_crypto_provider();
        let config = StreamableHttpClientTransportConfig::with_uri(format!("http://{address}/mcp"))
            .auth_header(token.to_owned());
        ().serve(StreamableHttpClientTransport::from_config(config))
            .await
            .map_err(|e| fail("mcp client", e))
    }
}

/// Tham số gọi một tool MCP: `tool("notes.create", json!({"title": "…"}))`.
pub fn tool(name: &str, arguments: Value) -> CallToolRequestParams {
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    CallToolRequestParams::new(name.to_owned()).with_arguments(arguments)
}

impl<'a> TestAppBuilder<'a> {
    /// Đặt một biến cấu hình (như biến môi trường). `DATABASE_URL` đã có sẵn.
    pub fn setting(mut self, name: &str, value: &str) -> Self {
        self.settings.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Thêm một module. Thứ tự như `crates/app/src/modules.rs`: bên cung cấp trait trước bên
    /// dùng.
    pub fn module(mut self, module: impl Module + 'static) -> Self {
        self.modules.push(Box::new(module));
        self
    }

    /// Công bố một trait thay cho module thật, vd `provide::<dyn Storage>(Arc::new(fake))`.
    /// Mọi thứ `provide` được công bố bởi module `testkit`, đứng trước mọi module khác.
    pub fn provide<T>(mut self, api: Arc<T>) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided.push(Box::new(move |context| context.provide::<T>(api)));
        self
    }

    /// Thêm một người dùng giả: request mang `Authorization: Bearer <token>` được xác thực
    /// là `user_id` với đúng các permission này. Khai ít nhất một `user` (hay `principal`) thì
    /// app dùng bộ xác thực giả thay cho bộ một module công bố.
    pub fn user(self, token: &str, user_id: u128, permissions: &[&str]) -> Self {
        let permissions = permissions.iter().map(|p| (*p).to_owned());
        self.principal(token, Principal::new(Uuid::from_u128(user_id), permissions))
    }

    /// Như [`TestAppBuilder::user`], với `Principal` dựng sẵn — cho bên gọi mà `user` không tả
    /// được, vd gọi bằng API key: `Principal::new(id, permissions).via_api_key(key_id)`.
    pub fn principal(mut self, token: &str, principal: Principal) -> Self {
        self.users.insert(token.to_owned(), principal);
        self
    }

    /// Đọc cấu hình, chạy mọi migration, khởi động module, ghép router.
    pub async fn build(self) -> Result<TestApp, TestAppError> {
        let mut pairs = vec![("DATABASE_URL".to_owned(), self.test.url().to_owned())];
        pairs.extend(self.settings);
        let source = ConfigSource::from_pairs(pairs);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);

        let mut modules: Vec<Box<dyn Module>> = Vec::new();
        if !self.provided.is_empty() {
            modules.push(Box::new(Provider(self.provided)));
        }
        modules.extend(self.modules);
        let mut modules = Modules::new(modules).map_err(|e| fail("modules", e))?;
        let configured = modules.configure(&mut reader);
        let (config, _) = reader
            .finish(config.and_then(|config| configured.map(|()| config)))
            .map_err(|e| fail("configuration", e))?;

        let database =
            Database::connect(&config.database).await.map_err(|e| fail("database", e))?;
        let migrator = modules.migrator().map_err(|e| fail("migrations", e))?;
        migrator.up(&database).await.map_err(|e| fail("migrations", e))?;
        modules.start(&database).map_err(|e| fail("start", e))?;

        let authenticator: Option<Arc<dyn Authenticator>> = if self.users.is_empty() {
            modules.api::<dyn Authenticator>()
        } else {
            Some(Arc::new(FakeAuthenticator(self.users)))
        };
        let extras = Extras {
            authenticator,
            idempotency: modules.api::<dyn IdempotencyStore>(),
            deprecations: modules.deprecations(),
            body_limits: modules.body_limits(),
            tools: modules.tools().map_err(|e| fail("tools", e))?,
            translations: modules.translations().clone(),
        };
        let root = http::health_routes(database.clone());
        let api = http::build(&config.http, root, modules.routes(), extras);
        if !api.schema_conflicts.is_empty() {
            return Err(fail("openapi", api.schema_conflicts.join(", ")));
        }
        Ok(TestApp { router: api.router, openapi: api.openapi, modules, database })
    }
}

/// Module `testkit`: công bố những gì test đưa qua [`TestAppBuilder::provide`].
struct Provider(Vec<Provision>);

impl Module for Provider {
    fn name(&self) -> &'static str {
        "testkit"
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        for provide in std::mem::take(&mut self.0) {
            provide(context)?;
        }
        Ok(())
    }
}

/// Bộ xác thực giả: token → bên gọi khai bằng [`TestAppBuilder::user`] hay
/// [`TestAppBuilder::principal`].
struct FakeAuthenticator(HashMap<String, Principal>);

#[async_trait::async_trait]
impl Authenticator for FakeAuthenticator {
    async fn authenticate(&self, authorization: Option<&str>) -> Result<Principal, Error> {
        let user = authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| self.0.get(token));
        match user {
            Some(principal) => Ok(principal.clone()),
            None => Err(Error::unauthenticated("auth.invalid_token", "The token is not valid.")),
        }
    }
}

/// Request đang dựng. Gửi bằng [`TestRequest::send`].
pub struct TestRequest<'a> {
    app: &'a TestApp,
    builder: axum::http::request::Builder,
    body: Option<Body>,
}

impl TestRequest<'_> {
    /// `Authorization: Bearer <token>`.
    pub fn token(self, token: &str) -> Self {
        self.header(header::AUTHORIZATION.as_str(), &format!("Bearer {token}"))
    }

    /// Thêm một header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.builder = self.builder.header(name, value);
        self
    }

    /// Thân JSON (kèm `Content-Type: application/json`).
    pub fn json(self, body: Value) -> Self {
        let mut request = self.header(header::CONTENT_TYPE.as_str(), "application/json");
        request.body = Some(Body::from(body.to_string()));
        request
    }

    /// Thân thô (tự đặt `Content-Type` bằng [`TestRequest::header`]).
    pub fn bytes(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(Body::from(body.into()));
        self
    }

    /// Thân bất kỳ, vd `Body::from_stream(…)` để gửi theo từng đoạn, không có
    /// `Content-Length` (tự đặt `Content-Type` bằng [`TestRequest::header`]).
    pub fn body(mut self, body: Body) -> Self {
        self.body = Some(body);
        self
    }

    /// Gửi qua router (không mở cổng mạng) và đọc hết câu trả lời.
    pub async fn send(self) -> Result<TestResponse, TestAppError> {
        let request = self
            .builder
            .body(self.body.unwrap_or_else(Body::empty))
            .map_err(|e| fail("request", e))?;
        let response =
            self.app.router.clone().oneshot(request).await.map_err(|e| fail("send", e))?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|e| fail("body", e))?;
        Ok(TestResponse { status, headers, body: body.to_vec() })
    }
}

/// Câu trả lời đã đọc hết thân.
#[derive(Debug)]
pub struct TestResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl TestResponse {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Giá trị header dạng chuỗi; `""` khi không có hoặc không phải ASCII.
    pub fn header(&self, name: impl TryInto<HeaderName>) -> &str {
        name.try_into()
            .ok()
            .and_then(|name| self.headers.get(name))
            .and_then(|value: &HeaderValue| value.to_str().ok())
            .unwrap_or("")
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Thân dạng JSON; `Value::Null` khi rỗng hoặc không phải JSON.
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// Thân dạng chữ (UTF-8, ký tự hỏng được thay).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}
