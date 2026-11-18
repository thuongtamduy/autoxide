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
//!
//! Nhiều khách (bước 33): `.tenant("a")` thêm khách trên database của app (placement chung),
//! `.tenant_dedicated("b")` thêm khách trên database riêng (một database trống nữa cùng loại);
//! `.placement("p")` thêm placement riêng chưa có khách (đích để dời khách tới);
//! `.user_in("a", "token-a", 1, &[…])` là người của khách `a`. Khai khách nào thì app chạy chế độ
//! `multi` (bỏ qua việc chế độ này chưa mở cho người dùng); `app.tenant("a")` cho ngữ cảnh của
//! khách để gọi service trực tiếp.
//!
//! Tên miền riêng (D-190): `.domain("a", "shop-a.test")` gắn tên miền đã xác minh cho khách `a`;
//! `app.get(…).host("shop-a.test")` gửi request tới tên miền đó. App không bao giờ hỏi DNS thật:
//! bản ghi TXT đặt ở `app.dns()` ([`FakeDns`]).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use autoxide_kernel::access::{Authenticator, Grant, Principal};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{self, Extras};
use autoxide_kernel::idempotency::IdempotencyStore;
use autoxide_kernel::module::{Module, Modules, StartContext};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, NewPlacement, NewTenant, OrgId, PlacementKind, TenancyConfig, TenancyMode,
    TenantAdmin, TenantContext, TenantDomains,
};
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
use crate::dns::FakeDns;

/// Lỗi khi dựng hay gọi [`TestApp`]. Thông báo nói bước nào hỏng.
#[derive(Debug)]
pub struct TestAppError(String);

impl fmt::Display for TestAppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "test app: {}", self.0)
    }
}

impl std::error::Error for TestAppError {}

impl TestAppError {
    pub(crate) fn new(message: String) -> Self {
        TestAppError(message)
    }
}

fn fail(step: &str, error: impl fmt::Display) -> TestAppError {
    TestAppError(format!("{step}: {error}"))
}

/// Ứng dụng dựng sẵn cho một bài test. Lấy qua [`TestApp::builder`].
pub struct TestApp {
    router: Router,
    openapi: OpenApi,
    /// `openapi` dạng JSON, cho bộ kiểm response theo hợp đồng.
    contract: Value,
    // Giữ module sống cùng app: chúng sở hữu service của mình.
    modules: Modules,
    database: Database,
    registry: DatabaseRegistry,
    dns: FakeDns,
}

/// Cấu hình của [`TestApp`] trước khi dựng.
pub struct TestAppBuilder<'a> {
    test: &'a TestDatabase,
    settings: Vec<(String, String)>,
    modules: Vec<Box<dyn Module>>,
    provided: Vec<Provision>,
    users: HashMap<String, Principal>,
    multi: bool,
    /// Khách khai thêm: (slug, có database riêng không).
    tenants: Vec<(String, bool)>,
    /// Placement riêng chưa có khách nào (đích để dời khách tới).
    placements: Vec<String>,
    /// Người của khách: (slug, token, bên gọi).
    tenant_users: Vec<(String, String, Principal)>,
    /// Tên miền đã xác minh của khách: (slug, tên miền).
    domains: Vec<(String, String)>,
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
            multi: false,
            tenants: Vec::new(),
            placements: Vec::new(),
            tenant_users: Vec::new(),
            domains: Vec::new(),
        }
    }

    /// Sổ database của app (chế độ, placement, tra khách).
    pub fn registry(&self) -> &DatabaseRegistry {
        &self.registry
    }

    /// Khách mặc định (khách nền tảng) trên database của app.
    pub fn default_tenant(&self) -> TenantContext {
        self.registry.default_tenant()
    }

    /// Ngữ cảnh của khách đã khai bằng [`TestAppBuilder::tenant`] /
    /// [`TestAppBuilder::tenant_dedicated`] (theo slug).
    pub async fn tenant(&self, slug: &str) -> Result<TenantContext, TestAppError> {
        self.registry.tenant_by_slug(slug).await.map_err(|e| fail("tenant", e))
    }

    /// Bộ tra DNS giả của app: tên miền riêng được xác minh theo bản ghi TXT đặt ở đây
    /// ([`FakeDns::set_challenge`]); app không bao giờ hỏi DNS thật.
    pub fn dns(&self) -> &FakeDns {
        &self.dns
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
        TestRequest {
            app: self,
            method: method.to_string(),
            path: uri.split('?').next().unwrap_or(uri).to_owned(),
            checked: true,
            builder: Request::builder().method(method).uri(uri),
            body: None,
        }
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

    /// Như [`module`](Self::module), cho module đã đóng hộp — vd cả danh sách module của binary
    /// (`modules::all()`) trong test đầu-cuối.
    pub fn boxed_module(mut self, module: Box<dyn Module>) -> Self {
        self.modules.push(module);
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

    /// Góp một bộ cài đặt cho trait nhiều bên cùng cung cấp (`StartContext::contribute`), vd
    /// `.contribute::<dyn PaymentGateway>(Arc::new(FakePaymentGateway::new()))`.
    pub fn contribute<T>(mut self, api: Arc<T>) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided.push(Box::new(move |context| context.contribute::<T>(api)));
        self
    }

    /// Thêm một người dùng giả: request mang `Authorization: Bearer <token>` được xác thực
    /// là `user_id` với đúng các permission này. Khai ít nhất một `user` (hay `principal`) thì
    /// app dùng bộ xác thực giả thay cho bộ một module công bố.
    pub fn user(self, token: &str, user_id: u128, permissions: &[&str]) -> Self {
        let permissions = permissions.iter().map(|p| (*p).to_owned());
        self.principal(token, Principal::new(Uuid::from_u128(user_id), permissions))
    }

    /// Người dùng giả có quyền theo nút (bước 34): mỗi cặp `(permission, độ sâu)` là một grant ở
    /// nút `org` của khách mặc định, vd `.member("ann", 1, shop, &[("notes.read", Scope::Unit)])`.
    /// Cây (nút, nhánh) dựng bằng `autoxide_kernel::tenancy::OrgTree` trong bài test.
    pub fn member(
        self,
        token: &str,
        user_id: u128,
        org: OrgId,
        grants: &[(&str, autoxide_kernel::access::Scope)],
    ) -> Self {
        let grants = grants
            .iter()
            .map(|(permission, scope)| ((*permission).to_owned(), Grant::new(org, *scope)));
        self.principal(token, Principal::with_grants(Uuid::from_u128(user_id), grants))
    }

    /// Như [`TestAppBuilder::user`], với `Principal` dựng sẵn — cho bên gọi mà `user` không tả
    /// được, vd gọi bằng API key: `Principal::new(id, permissions).via_api_key(key_id)`.
    pub fn principal(mut self, token: &str, principal: Principal) -> Self {
        self.users.insert(token.to_owned(), principal);
        self
    }

    /// Chạy chế độ nhiều khách (`AUTOXIDE_TENANCY_MODE=multi`) dù chưa khai khách nào.
    pub fn multi_tenant(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Thêm khách `slug` trên database của app (placement chung `main`). Bật chế độ nhiều khách.
    pub fn tenant(mut self, slug: &str) -> Self {
        self.multi = true;
        self.tenants.push((slug.to_owned(), false));
        self
    }

    /// Thêm khách `slug` trên database riêng (placement `dedicated` tên `slug`, một database trống
    /// cùng loại, dọn cùng bài test). Bật chế độ nhiều khách.
    pub fn tenant_dedicated(mut self, slug: &str) -> Self {
        self.multi = true;
        self.tenants.push((slug.to_owned(), true));
        self
    }

    /// Thêm placement `dedicated` tên `name` trên một database trống nữa cùng loại (dọn cùng bài
    /// test), chưa có khách nào — vd đích để dời khách tới. Bật chế độ nhiều khách.
    pub fn placement(mut self, name: &str) -> Self {
        self.multi = true;
        self.placements.push(name.to_owned());
        self
    }

    /// Như [`user`](Self::user), cho người của khách `slug` (khai bằng [`tenant`](Self::tenant)
    /// hay [`tenant_dedicated`](Self::tenant_dedicated)).
    pub fn user_in(mut self, slug: &str, token: &str, user_id: u128, permissions: &[&str]) -> Self {
        let permissions = permissions.iter().map(|p| (*p).to_owned());
        let principal = Principal::new(Uuid::from_u128(user_id), permissions);
        self.tenant_users.push((slug.to_owned(), token.to_owned(), principal));
        self
    }

    /// Gắn tên miền `domain` (đã xác minh) cho khách `slug` (khai bằng [`tenant`](Self::tenant),
    /// hay `default` cho khách nền tảng): request mang `Host: <domain>` vào khách đó. Đi đúng đường
    /// thật — thêm, đặt bản ghi TXT ở [`FakeDns`], xác minh. Bật chế độ nhiều khách.
    pub fn domain(mut self, slug: &str, domain: &str) -> Self {
        self.multi = true;
        self.domains.push((slug.to_owned(), domain.to_owned()));
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
        let registry = if self.multi {
            // Đặt thẳng chế độ (đặt `AUTOXIDE_TENANCY_MODE=multi` thì phải có cả khoá gốc); các
            // biến `AUTOXIDE_TENANCY_*` khác test đặt bằng `setting` vẫn có tác dụng.
            let mut tenancy: TenancyConfig = config.tenancy.clone();
            tenancy.mode = TenancyMode::Multi;
            DatabaseRegistry::new(database.clone(), config.database.clone(), tenancy)
        } else {
            DatabaseRegistry::single(database.clone())
        };
        let dns = FakeDns::default();
        modules.use_dns(Arc::new(dns.clone()));
        modules.start_in(&registry).map_err(|e| fail("start", e))?;

        let mut users = self.users;
        if !self.tenants.is_empty() || !self.placements.is_empty() {
            let placements = modules.placements(&registry).map_err(|e| fail("placements", e))?;
            let dedicated = self.tenants.iter().filter(|(_, dedicated)| *dedicated);
            let names = dedicated.map(|(slug, _)| slug).chain(&self.placements);
            for name in names {
                let url = self.test.extra_database().await.map_err(|e| fail("placement", e))?;
                let url = url.parse().map_err(|e| fail("placement", e))?;
                let new = NewPlacement::new(name.clone(), PlacementKind::Dedicated, url);
                placements.add(new).await.map_err(|e| fail("placement", e))?;
            }
            placements.up_all().await.map_err(|e| fail("placement migrations", e))?;
            let admin = modules
                .api::<TenantAdmin>()
                .ok_or_else(|| fail("tenants", "the kernel did not start its tenant service"))?;
            for (slug, dedicated) in &self.tenants {
                let input = NewTenant {
                    slug: slug.clone(),
                    name: slug.clone(),
                    parent_id: None,
                    placement: dedicated.then(|| slug.clone()),
                };
                let actor = autoxide_kernel::event::Actor::system();
                admin.create(actor, input).await.map_err(|e| fail("tenant", e))?;
            }
        }
        if !self.domains.is_empty() {
            let domains = modules
                .api::<TenantDomains>()
                .ok_or_else(|| fail("domains", "the kernel did not start its domain service"))?;
            for (slug, domain) in &self.domains {
                let tenant = registry.tenant_by_slug(slug).await.map_err(|e| fail("domain", e))?;
                let actor = autoxide_kernel::event::Actor::system;
                let added =
                    domains.add(actor(), &tenant, domain).await.map_err(|e| fail("domain", e))?;
                dns.set_txt(&added.record.name, &[&added.record.value]);
                let verified = domains
                    .verify(actor(), &tenant, added.id)
                    .await
                    .map_err(|e| fail("domain", e))?;
                if verified.status != "verified" {
                    return Err(fail("domain", format!("`{domain}` was not verified")));
                }
            }
        }
        for (slug, token, principal) in self.tenant_users {
            let tenant = registry.tenant_by_slug(&slug).await.map_err(|e| fail("user_in", e))?;
            users.insert(token, principal.in_tenant(tenant.id()));
        }

        let authenticator: Option<Arc<dyn Authenticator>> = if users.is_empty() {
            modules.api::<dyn Authenticator>()
        } else {
            Some(Arc::new(FakeAuthenticator(users)))
        };
        let mut extras = Extras::default()
            .authenticator(authenticator)
            .idempotency(modules.api::<dyn IdempotencyStore>())
            .deprecations(modules.deprecations())
            .body_limits(modules.body_limits())
            .tools(modules.tools().map_err(|e| fail("tools", e))?)
            .resources(modules.mcp_resources().map_err(|e| fail("resources", e))?)
            .prompts(modules.prompts().map_err(|e| fail("prompts", e))?)
            .modules(modules.descriptions())
            .public_url(
                modules
                    .api::<autoxide_kernel::public_url::PublicUrl>()
                    .and_then(|url| url.base().ok().map(str::to_owned)),
            )
            .entitlements(modules.entitlements())
            .translations(modules.translations().clone());
        if let Some(registry) = modules.tenancy() {
            extras = extras.tenancy(registry);
        }
        let root = http::health_routes(database.clone(), http::Draining::default());
        let api = http::build(&config.http, root, modules.routes(), extras);
        if !api.schema_conflicts.is_empty() {
            return Err(fail("openapi", api.schema_conflicts.join(", ")));
        }
        let contract = serde_json::to_value(&api.openapi).map_err(|e| fail("openapi", e))?;
        Ok(TestApp {
            router: api.router,
            openapi: api.openapi,
            contract,
            modules,
            database,
            registry,
            dns,
        })
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
    async fn authenticate(
        &self,
        authorization: Option<&str>,
        _: &autoxide_kernel::tenancy::TenantHint,
    ) -> Result<Principal, Error> {
        let user = authorization
            .and_then(|value| value.strip_prefix("Bearer "))
            .and_then(|token| self.0.get(token));
        match user {
            Some(principal) => Ok(principal.clone()),
            None => Err(Error::unauthenticated("auth.invalid_token", "The token is not valid.")),
        }
    }

    // Người dùng của test theo ID (cùng khách): như bộ xác thực thật tra quyền của người khác.
    async fn principal_of(
        &self,
        tenant: &autoxide_kernel::tenancy::TenantContext,
        user_id: uuid::Uuid,
    ) -> Result<Option<Principal>, Error> {
        Ok(self
            .0
            .values()
            .find(|principal| {
                principal.user_id() == user_id && principal.tenant_id() == tenant.id()
            })
            .cloned())
    }
}

/// Request đang dựng. Gửi bằng [`TestRequest::send`].
pub struct TestRequest<'a> {
    app: &'a TestApp,
    method: String,
    /// Đường dẫn, bỏ phần query.
    path: String,
    /// Kiểm response theo hợp đồng OpenAPI (mặc định có).
    checked: bool,
    builder: axum::http::request::Builder,
    body: Option<Body>,
}

impl TestRequest<'_> {
    /// `Authorization: Bearer <token>`.
    pub fn token(self, token: &str) -> Self {
        self.header(header::AUTHORIZATION.as_str(), &format!("Bearer {token}"))
    }

    /// `Host: <host>` — request tới tên miền riêng của khách (khai bằng
    /// [`TestAppBuilder::domain`]).
    pub fn host(self, host: &str) -> Self {
        self.header(header::HOST.as_str(), host)
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

    /// Không kiểm response theo hợp đồng OpenAPI — chỉ cho test cố ý tạo câu trả lời ngoài hợp
    /// đồng. Lệch hợp đồng thường là lỗi thật (tài liệu hay code): sửa nó thay vì gọi hàm này.
    pub fn unchecked(mut self) -> Self {
        self.checked = false;
        self
    }

    /// Gửi qua router (không mở cổng mạng) và đọc hết câu trả lời. Response lệch hợp đồng
    /// OpenAPI của endpoint (mã trạng thái không khai, media type sai, thân sai schema) là lỗi
    /// (đặc tả 31 B4) — trừ khi gọi [`unchecked`](Self::unchecked).
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
        if self.checked {
            let content_type =
                headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok());
            crate::contract::check(
                &self.app.contract,
                &self.method,
                &self.path,
                status.as_u16(),
                content_type,
                &body,
            )
            .map_err(|e| fail("response does not match docs/openapi.json", e))?;
        }
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
