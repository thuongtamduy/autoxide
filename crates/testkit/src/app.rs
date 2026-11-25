//! [`TestApp`]: builds modules the way the binary does (config → migration → `start` → kernel
//! router) on a [`TestDatabase`], then calls them over real REST or an MCP client.
//!
//! Previously every module rewrote about 60–100 lines to build the app, a fake authenticator and a request
//! helper (D-164). Now it takes only:
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
//! Built like `crates/app/src/runtime.rs`: same authenticator (published by a module, or a fake one when the
//! test declares a `user`), same idempotency, deprecation, request body limit, MCP tools and translations.
//!
//! Multiple tenants (step 33): `.tenant("a")` adds a tenant on the app's database (shared placement),
//! `.tenant_dedicated("b")` adds a tenant on its own database (one more empty database of the same kind);
//! `.placement("p")` adds a dedicated placement with no tenant yet (a target to move a tenant to);
//! `.user_in("a", "token-a", 1, &[…])` is a user of tenant `a`. Declaring any tenant runs the app in `multi`
//! mode (bypassing the fact that this mode is not yet open to users); `app.tenant("a")` gives the tenant's
//! context for calling services directly.
//!
//! Custom domains (D-190): `.domain("a", "shop-a.test")` attaches a verified domain to tenant `a`;
//! `app.get(…).host("shop-a.test")` sends a request to that domain. The app never queries real DNS:
//! the TXT record is set on `app.dns()` ([`FakeDns`]).

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use crate::statements::StatementLog;

use crate::mcp::{CallToolRequestParams, McpClient};
use autoxide_kernel::access::{Authenticator, Grant, Principal};
use autoxide_kernel::config::{Config, ConfigReader, ConfigSource};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::{self, Extras};
use autoxide_kernel::idempotency::IdempotencyStore;
use autoxide_kernel::module::{Module, Modules, Selection, StartContext};
use autoxide_kernel::tenancy::{
    DatabaseRegistry, NewPlacement, NewTenant, OrgId, PlacementKind, TenancyConfig, TenancyMode,
    TenantAdmin, TenantContext, TenantDomains,
};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt as _;
use utoipa::openapi::OpenApi;
use uuid::Uuid;

use crate::TestDatabase;
use crate::dns::FakeDns;

/// Error while building or calling a [`TestApp`]. The message says which step failed.
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

/// A prebuilt application for one test. Obtained through [`TestApp::builder`].
pub struct TestApp {
    router: Router,
    openapi: OpenApi,
    /// `openapi` as JSON, for the contract-based response checker.
    contract: Value,
    // Giữ module sống cùng app: chúng sở hữu service của mình.
    modules: Modules,
    database: Database,
    registry: DatabaseRegistry,
    dns: FakeDns,
    /// The database was copied from the template database instead of migrated from scratch.
    from_template: bool,
    statements: Arc<StatementLog>,
    /// Temporary directory for uploaded files and the app's mail files; deleted when the `TestApp` is dropped.
    _scratch: Option<ScratchDir>,
}

/// Temporary directory that deletes itself when dropped (`Drop`). Rust calls `drop` when a value leaves scope —
/// like `finally` — so a test leaves no files behind whether it passes or fails.
struct ScratchDir(std::path::PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        // Lỗi xoá (thư mục chưa từng được tạo, đã mất) không đáng làm test hỏng.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Configuration of a [`TestApp`] before it is built.
pub struct TestAppBuilder<'a> {
    test: &'a TestDatabase,
    settings: Vec<(String, String)>,
    modules: Vec<Box<dyn Module>>,
    /// Modules present in the program but turned off ([`TestAppBuilder::module_turned_off`]).
    turned_off: Vec<Box<dyn Module>>,
    provided: Vec<Provision>,
    users: HashMap<String, Principal>,
    multi: bool,
    /// Extra tenants declared: (slug, has its own database, slug of the parent tenant).
    tenants: Vec<(String, bool, Option<String>)>,
    /// Dedicated placements with no tenant yet (a target to move a tenant to).
    placements: Vec<String>,
    /// Tenant users: (slug, token, caller).
    tenant_users: Vec<(String, String, Principal)>,
    /// Verified domains of tenants: (slug, domain).
    domains: Vec<(String, String)>,
    /// Migrate on an empty database instead of copying the template database ([`TestAppBuilder::empty_database`]).
    empty_database: bool,
}

// `for<'c>`: hàm nhận `StartContext` với lifetime bất kỳ — lifetime thật chỉ biết lúc
// `start` chạy, không phải lúc test gọi `provide`.
type Provision = Box<dyn for<'c> FnOnce(&mut StartContext<'c>) -> Result<(), Error> + Send + Sync>;

impl TestApp {
    /// Starts building the app on the test's database.
    pub fn builder(test: &TestDatabase) -> TestAppBuilder<'_> {
        TestAppBuilder {
            test,
            settings: Vec::new(),
            modules: Vec::new(),
            turned_off: Vec::new(),
            provided: Vec::new(),
            users: HashMap::new(),
            multi: false,
            tenants: Vec::new(),
            placements: Vec::new(),
            tenant_users: Vec::new(),
            domains: Vec::new(),
            empty_database: false,
        }
    }

    /// The app's database is copied from the migrated template database (D-20261120-test-db-template) instead
    /// of being migrated from scratch. Only for the testkit's own self-checks; tests do not need to know.
    pub fn from_template(&self) -> bool {
        self.from_template
    }

    /// Runs `work` and returns every SQL statement the app ran meanwhile (D-20261120-step39-hot-paths) —
    /// so a test can pin the statement count of a request. Only one measurement at a time; with nested
    /// measurements the inner one takes all the statements.
    pub async fn statements<T>(&self, work: impl Future<Output = T>) -> (T, Vec<String>) {
        self.statements.start();
        let output = work.await;
        (output, self.statements.stop())
    }

    /// The app's database registry (mode, placement, tenant lookup).
    pub fn registry(&self) -> &DatabaseRegistry {
        &self.registry
    }

    /// The default tenant (the platform tenant) on the app's database.
    pub fn default_tenant(&self) -> TenantContext {
        self.registry.default_tenant()
    }

    /// Context of a tenant declared with [`TestAppBuilder::tenant`] /
    /// [`TestAppBuilder::tenant_dedicated`] (by slug).
    pub async fn tenant(&self, slug: &str) -> Result<TenantContext, TestAppError> {
        self.registry.tenant_by_slug(slug).await.map_err(|e| fail("tenant", e))
    }

    /// Treats every unfinished tenant move as having passed the wait for other servers to see the move
    /// (cache TTL + the maximum duration of a request): the copy step runs immediately.
    ///
    /// Call after `TenantMoves::start`, before `run` / `step` / the worker. This way a test need not lower
    /// `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS` to 1 second just to get a short wait — that value
    /// makes ordinary test requests occasionally get cut off (408) when the CI machine is slow.
    pub async fn skip_move_wait(&self) -> Result<(), TestAppError> {
        use sea_orm::ConnectionTrait as _;
        use sea_orm::sea_query::{Alias, Expr, ExprTrait as _, Query};

        // Lùi `moving_since` một giờ: xa hơn mọi thời gian chờ test đặt, nhưng không đổi gì khác
        // của lượt dời (trạng thái, lease, bảng đã chép).
        let earlier = Timestamp::now() - SignedDuration::from_hours(1);
        let update = Query::update()
            .table(Alias::new("kernel_tenant_moves"))
            .value(Alias::new("moving_since"), earlier)
            .and_where(Expr::col(Alias::new("finished_at")).is_null())
            .to_owned();
        self.registry
            .control()
            .write()
            .execute(&update)
            .await
            .map_err(|e| fail("skip_move_wait", e))?;
        Ok(())
    }

    /// Like [`skip_move_wait`](Self::skip_move_wait), for tenant split / merge moves: the freeze step does not
    /// wait for the cache TTL + the maximum request duration. Call after `build`; no need to lower
    /// `AUTOXIDE_HTTP_REQUEST_TIMEOUT_SECONDS`.
    pub fn skip_relocation_wait(&self) -> Result<(), TestAppError> {
        let relocations = self
            .api::<autoxide_kernel::tenancy::TenantRelocations>()
            .ok_or_else(|| fail("skip_relocation_wait", "the kernel has no relocation service"))?;
        relocations.skip_settle();
        Ok(())
    }

    /// The app's fake DNS resolver: a custom domain is verified by the TXT record set here
    /// ([`FakeDns::set_challenge`]); the app never queries real DNS.
    pub fn dns(&self) -> &FakeDns {
        &self.dns
    }

    /// The full router (every HTTP layer of the kernel), for sending requests yourself when the helpers are not enough.
    pub fn router(&self) -> Router {
        self.router.clone()
    }

    /// The app's OpenAPI document.
    pub fn openapi(&self) -> &OpenApi {
        &self.openapi
    }

    /// The started modules (fetch published traits, workers, dispatchers…).
    pub fn modules(&self) -> &Modules {
        &self.modules
    }

    /// The app's database (read to check results, run jobs).
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// A trait a module has published (`context.provide`), e.g. the module's own service.
    pub fn api<T>(&self) -> Option<Arc<T>>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.modules.api::<T>()
    }

    /// The block registry of the modules in the app (step 67): a test calls a module's `BlockData` the way
    /// `content` does on a public read.
    pub fn blocks(&self) -> autoxide_kernel::blocks::Blocks {
        self.modules.blocks()
    }

    /// Starts a request. `uri` is the full path, e.g. `/api/v1/notes/{id}`.
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

    /// An MCP client connected to the app's `/mcp`, authenticated with `token` (like [`TestRequest::token`]).
    /// Speaks the new spec version (`2026-07-28`); for the old one (`initialize`) use [`McpClient::legacy`]
    /// with [`TestApp::router`].
    pub async fn mcp(&self, token: &str) -> Result<McpClient, TestAppError> {
        self.mcp_at(token, "/mcp").await
    }

    /// Like [`TestApp::mcp`], to the MCP endpoint at `path` — e.g. `/mcp/notes`, a module's own
    /// endpoint.
    ///
    /// After connecting it calls `server/discover` (as a client does in its handshake): a caller that cannot
    /// reach the endpoint (wrong token, locked tenant…) fails right here.
    pub async fn mcp_at(&self, token: &str, path: &str) -> Result<McpClient, TestAppError> {
        let client = McpClient::new(self.router.clone(), path, Some(token));
        client.discover().await.map_err(|e| fail("mcp", e))?;
        Ok(client)
    }
}

/// Parameters for calling an MCP tool: `tool("notes.create", json!({"title": "…"}))`.
pub fn tool(name: &str, arguments: Value) -> CallToolRequestParams {
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    CallToolRequestParams::new(name.to_owned()).with_arguments(arguments)
}

impl<'a> TestAppBuilder<'a> {
    /// Sets a configuration variable (like an environment variable). `DATABASE_URL` is already set.
    pub fn setting(mut self, name: &str, value: &str) -> Self {
        self.settings.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Adds a module. Order as in `crates/app/src/modules.rs`: the provider of a trait before the
    /// consumer.
    pub fn module(mut self, module: impl Module + 'static) -> Self {
        self.modules.push(Box::new(module));
        self
    }

    /// Like [`module`](Self::module), for an already boxed module — e.g. the binary's whole module list
    /// (`modules::all()`) in an end-to-end test.
    pub fn boxed_module(mut self, module: Box<dyn Module>) -> Self {
        self.modules.push(module);
        self
    }

    /// A module present in the program but OFF, as when `AUTOXIDE_MODULES` does not enable it: it does not
    /// start and its migrations run only where its tables already exist (none on the fresh test database);
    /// it takes part through `Module::dormant` (`Modules::inactive`). For testing what the program does with
    /// the data that module left behind.
    pub fn module_turned_off(mut self, module: impl Module + 'static) -> Self {
        self.turned_off.push(Box::new(module));
        self
    }

    /// Publishes a trait in place of a real module, e.g. `provide::<dyn Storage>(Arc::new(fake))`.
    /// Everything published with `provide` comes from the `testkit` module, ahead of every other module.
    pub fn provide<T>(mut self, api: Arc<T>) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided.push(Box::new(move |context| context.provide::<T>(api)));
        self
    }

    /// Contributes an implementation of a trait several parties provide together
    /// (`StartContext::contribute`), e.g.
    /// `.contribute::<dyn PaymentGateway>(Arc::new(FakePaymentGateway::new()))`.
    pub fn contribute<T>(mut self, api: Arc<T>) -> Self
    where
        T: ?Sized + Send + Sync + 'static,
    {
        self.provided.push(Box::new(move |context| context.contribute::<T>(api)));
        self
    }

    /// Adds a fake user: a request carrying `Authorization: Bearer <token>` is authenticated
    /// as `user_id` with exactly these permissions. Declaring at least one `user` (or `principal`) makes the
    /// app use the fake authenticator instead of the one a module publishes.
    pub fn user(self, token: &str, user_id: u128, permissions: &[&str]) -> Self {
        let permissions = permissions.iter().map(|p| (*p).to_owned());
        self.principal(token, Principal::new(Uuid::from_u128(user_id), permissions))
    }

    /// A fake user with per-node permissions (step 34): each `(permission, depth)` pair is a grant at the
    /// `org` node of the default tenant, e.g. `.member("ann", 1, shop, &[("notes.read", Scope::Unit)])`.
    /// Build the tree (node, branch) in the test with `autoxide_kernel::tenancy::OrgTree`.
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

    /// Like [`TestAppBuilder::user`], with a prebuilt `Principal` — for callers `user` cannot describe,
    /// e.g. an API key call: `Principal::new(id, permissions).via_api_key(key_id)`.
    pub fn principal(mut self, token: &str, principal: Principal) -> Self {
        self.users.insert(token.to_owned(), principal);
        self
    }

    /// Runs in multi-tenant mode (`AUTOXIDE_TENANCY_MODE=multi`) even when no tenant is declared.
    pub fn multi_tenant(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Adds tenant `slug` on the app's database (shared placement `main`). Turns on multi-tenant mode.
    pub fn tenant(mut self, slug: &str) -> Self {
        self.multi = true;
        self.tenants.push((slug.to_owned(), false, None));
        self
    }

    /// Adds tenant `slug` on its own database (placement `dedicated` named `slug`, one more empty database
    /// of the same kind, cleaned up with the test). Turns on multi-tenant mode.
    pub fn tenant_dedicated(mut self, slug: &str) -> Self {
        self.multi = true;
        self.tenants.push((slug.to_owned(), true, None));
        self
    }

    /// Adds tenant `slug` as a child of tenant `parent` (declared before it, or `default`) — the tenant tree
    /// of step 34; `dedicated`: on its own database like [`tenant_dedicated`](Self::tenant_dedicated).
    /// Turns on multi-tenant mode.
    pub fn child_tenant(mut self, slug: &str, parent: &str, dedicated: bool) -> Self {
        self.multi = true;
        self.tenants.push((slug.to_owned(), dedicated, Some(parent.to_owned())));
        self
    }

    /// Adds a `dedicated` placement named `name` on one more empty database of the same kind (cleaned up with
    /// the test), with no tenant yet — e.g. a target to move a tenant to. Turns on multi-tenant mode.
    pub fn placement(mut self, name: &str) -> Self {
        self.multi = true;
        self.placements.push(name.to_owned());
        self
    }

    /// Like [`user`](Self::user), for a user of tenant `slug` (declared with [`tenant`](Self::tenant)
    /// or [`tenant_dedicated`](Self::tenant_dedicated)).
    pub fn user_in(mut self, slug: &str, token: &str, user_id: u128, permissions: &[&str]) -> Self {
        let permissions = permissions.iter().map(|p| (*p).to_owned());
        let principal = Principal::new(Uuid::from_u128(user_id), permissions);
        self.tenant_users.push((slug.to_owned(), token.to_owned(), principal));
        self
    }

    /// Like [`principal`](Self::principal), for a user of tenant `slug` (e.g. that tenant's per-node
    /// permissions: `Principal::with_grants(…)`).
    pub fn principal_in(mut self, slug: &str, token: &str, principal: Principal) -> Self {
        self.tenant_users.push((slug.to_owned(), token.to_owned(), principal));
        self
    }

    /// Attaches the (verified) domain `domain` to tenant `slug` (declared with [`tenant`](Self::tenant), or
    /// `default` for the platform tenant): a request carrying `Host: <domain>` reaches that tenant. Goes the
    /// real way — add, set the TXT record on [`FakeDns`], verify. Turns on multi-tenant mode.
    pub fn domain(mut self, slug: &str, domain: &str) -> Self {
        self.multi = true;
        self.domains.push((slug.to_owned(), domain.to_owned()));
        self
    }

    /// Runs every migration on the test's empty database, without copying the migrated template database
    /// (D-20261120-test-db-template) — for tests that need that very migration (e.g. measuring or checking
    /// migrations). Same result; only slower.
    pub fn empty_database(mut self) -> Self {
        self.empty_database = true;
        self
    }

    /// Reads configuration, runs every migration (copying the migrated template database when possible),
    /// starts the modules, assembles the router.
    pub async fn build(self) -> Result<TestApp, TestAppError> {
        let mut pairs = vec![("DATABASE_URL".to_owned(), self.test.url().to_owned())];
        pairs.extend(self.settings);
        // Mặc định của `files` và `mail` là thư mục tương đối `data/files`, `data/mail` — tức ngay trong
        // thư mục chạy test (`crates/app`), và test upload để rác ở đó mãi. Bài test chưa chọn thư mục
        // (hay S3) thì cho một thư mục tạm riêng, xoá khi `TestApp` bị bỏ.
        let has = |pairs: &[(String, String)], name: &str| pairs.iter().any(|(n, _)| n == name);
        let mut scratch = None;
        let needs_files =
            !has(&pairs, "AUTOXIDE_FILES_DIR") && !has(&pairs, "AUTOXIDE_FILES_S3_BUCKET");
        let needs_mail = !has(&pairs, "AUTOXIDE_MAIL_FILE_DIR");
        if needs_files || needs_mail {
            let dir = std::env::temp_dir().join(format!("autoxide-testapp-{}", Uuid::now_v7()));
            if needs_files {
                pairs.push((
                    "AUTOXIDE_FILES_DIR".to_owned(),
                    dir.join("files").display().to_string(),
                ));
            }
            if needs_mail {
                pairs.push((
                    "AUTOXIDE_MAIL_FILE_DIR".to_owned(),
                    dir.join("mail").display().to_string(),
                ));
            }
            scratch = Some(ScratchDir(dir));
        }
        let source = ConfigSource::from_pairs(pairs);
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);

        let mut modules: Vec<Box<dyn Module>> = Vec::new();
        if !self.provided.is_empty() {
            modules.push(Box::new(Provider(self.provided)));
        }
        modules.extend(self.modules);
        let mut modules = if self.turned_off.is_empty() {
            // Không qua bộ chọn: test của một module thường thay module nó cần bằng bản giả
            // (`provide`), nên `Module::depends_on` không có module thật để trỏ tới.
            Modules::new(modules).map_err(|e| fail("modules", e))?
        } else {
            // Module tắt đi qua đúng bộ chọn của chương trình thật: `all` trừ từng tên.
            let mut selection = "all".to_owned();
            for module in &self.turned_off {
                selection.push_str(&format!(",-{}", module.name()));
            }
            modules.extend(self.turned_off);
            let selection = Selection::parse(&selection).map_err(|e| fail("modules", e))?;
            Modules::select(modules, &selection).map_err(|e| fail("modules", e))?
        };
        let configured = modules.configure(&mut reader);
        let (config, _) = reader
            .finish(config.and_then(|config| configured.map(|()| config)))
            .map_err(|e| fail("configuration", e))?;

        let migrator = modules.migrator().map_err(|e| fail("migrations", e))?;
        let mut from_template = false;
        if !self.empty_database {
            // Chép database mẫu đã migrate (D-20261120-test-db-template); không chép được thì
            // `up` bên dưới làm hết việc như trước.
            from_template = crate::template::prepare(&config.database, &migrator).await;
        }
        let mut database =
            Database::connect(&config.database).await.map_err(|e| fail("database", e))?;
        // Gắn bộ đếm câu SQL trước khi ai clone `database` (registry, module).
        let statements = Arc::new(StatementLog::default());
        database.observe_statements(statements.observer());
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
            let dedicated = self.tenants.iter().filter(|(_, dedicated, _)| *dedicated);
            let names = dedicated.map(|(slug, ..)| slug).chain(&self.placements);
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
            for (slug, dedicated, parent) in &self.tenants {
                let parent_id = match parent {
                    None => None,
                    Some(parent) => Some(
                        registry
                            .tenant_by_slug(parent)
                            .await
                            .map_err(|e| fail("child tenant", e))?
                            .id()
                            .as_uuid(),
                    ),
                };
                let input = NewTenant {
                    slug: slug.clone(),
                    name: slug.clone(),
                    parent_id,
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
            .experimental(modules.experimental())
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
            .sync(modules.sync_resources())
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
            from_template,
            statements,
            _scratch: scratch,
        })
    }
}

/// The `testkit` module: publishes what tests pass through [`TestAppBuilder::provide`].
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

/// Fake authenticator: token → the caller declared with [`TestAppBuilder::user`] or
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

/// A request being built. Sent with [`TestRequest::send`].
pub struct TestRequest<'a> {
    app: &'a TestApp,
    method: String,
    /// Path, without the query part.
    path: String,
    /// Check the response against the OpenAPI contract (on by default).
    checked: bool,
    builder: axum::http::request::Builder,
    body: Option<Body>,
}

impl TestRequest<'_> {
    /// `Authorization: Bearer <token>`.
    pub fn token(self, token: &str) -> Self {
        self.header(header::AUTHORIZATION.as_str(), &format!("Bearer {token}"))
    }

    /// `Host: <host>` — a request to a tenant's custom domain (declared with
    /// [`TestAppBuilder::domain`]).
    pub fn host(self, host: &str) -> Self {
        self.header(header::HOST.as_str(), host)
    }

    /// Adds a header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.builder = self.builder.header(name, value);
        self
    }

    /// JSON body (with `Content-Type: application/json`).
    pub fn json(self, body: Value) -> Self {
        let mut request = self.header(header::CONTENT_TYPE.as_str(), "application/json");
        request.body = Some(Body::from(body.to_string()));
        request
    }

    /// Raw body (set `Content-Type` yourself with [`TestRequest::header`]).
    pub fn bytes(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = Some(Body::from(body.into()));
        self
    }

    /// Any body, e.g. `Body::from_stream(…)` to send in chunks, with no
    /// `Content-Length` (set `Content-Type` yourself with [`TestRequest::header`]).
    pub fn body(mut self, body: Body) -> Self {
        self.body = Some(body);
        self
    }

    /// Does not check the response against the OpenAPI contract — only for tests that deliberately produce an
    /// answer outside the contract. A contract mismatch is usually a real bug (in the docs or the code): fix
    /// it instead of calling this.
    pub fn unchecked(mut self) -> Self {
        self.checked = false;
        self
    }

    /// Sends through the router (no network port opened) and reads the whole answer. A response that deviates
    /// from the endpoint's OpenAPI contract (undeclared status code, wrong media type, wrong body schema) is
    /// an error (spec 31 B4) — unless [`unchecked`](Self::unchecked) is called.
    pub async fn send(self) -> Result<TestResponse, TestAppError> {
        let request = self
            .builder
            .body(self.body.unwrap_or_else(Body::empty))
            .map_err(|e| fail("request", e))?;
        // Lỗi của `Router` là `Infallible` (không bao giờ xảy ra): `let Ok(…) = …` không cần nhánh lỗi.
        let Ok(response) = self.app.router.clone().oneshot(request).await;
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .map_err(|e| fail("body", e))?;
        if status.as_u16() < 400
            && let Some(template) =
                crate::contract::template(&self.app.contract, &self.method, &self.path)
        {
            crate::coverage::http(&self.method, template);
        }
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

/// An answer whose body has been fully read.
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

    /// Header value as a string; `""` when absent or not ASCII.
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

    /// Body as JSON; `Value::Null` when empty or not JSON.
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// Body as text (UTF-8, broken characters replaced).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}
