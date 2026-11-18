//! Máy chủ HTTP: ghép route của các module thành một router và bọc quanh nó các
//! lớp xử lý chung cho mọi request.
//!
//! Route của module `notes` nằm dưới `/api/v1/notes`. Số `v1` là phiên bản của
//! API: thay đổi phá vỡ thì mở `/api/v2`, không sửa `v1`.
//!
//! Tài liệu OpenAPI của mọi route nằm ở `/api/v1/openapi.json`, của từng module ở
//! `/api/v1/openapi/<module>.json`; trang xem tài liệu ở `/api/docs` khi được bật bằng cấu hình.
//!
//! Mọi request đi qua, theo thứ tự từ ngoài vào:
//!
//! 1. **request id** — mỗi request một ID (header `x-request-id`), trả lại trong
//!    response; client gửi sẵn ID hợp lệ thì dùng ID đó;
//! 2. **log** — mỗi request một dòng log khi xong; lỗi 5xx kèm nguyên nhân gốc;
//! 3. **CORS** — trình duyệt ở origin nào được gọi;
//! 4. **rate limiting** — mỗi địa chỉ client chỉ được gửi chừng ấy request mỗi phút;
//! 5. **idempotency** — lệnh ghi mang `Idempotency-Key` gửi lại thì nhận lại kết quả
//!    lần đầu, không làm hai lần;
//! 6. **ngôn ngữ** — `Accept-Language` chọn ngôn ngữ của `detail` trong lỗi;
//! 7. **chuẩn hoá lỗi** — mọi response lỗi đều là Problem Details (RFC 9457), kể
//!    cả lỗi do thư viện sinh ra (404, 405, 413…);
//! 8. **bắt panic** — handler panic thành lỗi 500 thay vì đứt kết nối;
//! 9. **timeout** — request chạy quá lâu bị cắt, trả 503;
//! 10. **giới hạn body** — body quá lớn bị từ chối, trả 413;
//! 11. **304** — `GET` có `ETag` khớp `If-None-Match` của client thành `304 Not Modified`.
//!
//! Ngoài lớp log (ngay trong lớp request id) là lớp **header bảo mật**: `nosniff`, `Referrer-Policy`,
//! `X-Frame-Options`, CSP chặt (HSTS khi cấu hình) cho mọi response — xem `security_headers`.

mod body_limit;
mod client;
mod conditional;
mod deprecation;
mod disposition;
mod entitlement;
mod extract;
mod health;
mod idempotency;
mod language;
mod llms;
mod log;
mod mcp;
mod metrics;
mod openapi;
mod openapi_modules;
mod problem;
mod rate_limit;
mod rate_limit_store;
mod request_id;
mod security_headers;
mod server;
pub(crate) mod stream;

use std::time::Duration;

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Router};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::request_id::{PropagateRequestIdLayer, SetRequestIdLayer};
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_scalar::{Scalar, Servable};

use crate::access::Authenticator;
use crate::config::{CorsOrigins, HttpConfig};
use crate::error::Error;
use crate::i18n::Translations;
use crate::idempotency::IdempotencyStore;

pub use body_limit::{BodyLimit, is_too_large};
pub use client::ClientInfo;
pub(crate) use client::ForwardedHost;
pub use deprecation::Deprecation;
pub use disposition::{Disposition, content_disposition};
pub use extract::ValidJson;
pub(crate) use extract::{contains_nul_escape, nul_refused, value_contains_nul};
pub use health::{Draining, Health, health_routes};
pub use llms::LLMS_PATH;
pub use mcp::MCP_PATH;
pub(crate) use mcp::{
    call_tool as call_mcp_tool, module_allowed as mcp_module_allowed,
    tool_allowed as mcp_tool_allowed,
};
pub use metrics::metrics_routes;
pub(crate) use openapi::untranslated_text;
pub use openapi::{BEARER_AUTH, DOCS_PATH, OPENAPI_PATH, lint};
pub use request_id::REQUEST_ID_HEADER;
pub use security_headers::{API_CSP, DOCS_CSP};
pub use server::{Shutdown, serve, shutdown_signal};

/// Tiền tố của mọi route API.
pub const API_PREFIX: &str = "/api/v1";

/// Router đã ghép xong, kèm tài liệu OpenAPI của mọi route trong nó.
#[non_exhaustive]
pub struct Api {
    pub router: Router,
    pub openapi: OpenApi,
    /// Tên schema bị hai nơi (module hay kernel) khai với nội dung khác nhau. Khi ghép tài liệu,
    /// utoipa giữ một bản và lặng lẽ bỏ bản kia — endpoint của một bên sẽ mô tả sai. Rỗng là
    /// đạt; lệnh `autoxide openapi` (và test giữ `docs/openapi.json` khớp code) coi đây là lỗi.
    pub schema_conflicts: Vec<String>,
}

/// Ghép route của các module (tên module, router) dưới [`API_PREFIX`], thêm các
/// route `root` ở gốc (ngoài `/api/v1`, vd [`health_routes`]), và bọc các lớp xử lý
/// chung quanh tất cả.
///
/// Những thứ dùng chung cho mọi request, ngoài route. Dựng bằng `Extras::default()` rồi các
/// hàm đặt từng trường.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct Extras {
    /// Bộ xác thực một module đã công bố; handler nhận
    /// [`Principal`](crate::access::Principal) dùng nó. `None`: chưa module nào xác
    /// thực — handler cần đăng nhập trả lỗi 500 nói rõ thiếu gì.
    pub authenticator: Option<Arc<dyn Authenticator>>,
    /// Nơi lưu idempotency key (một module cung cấp). `None`: header
    /// `Idempotency-Key` bị bỏ qua.
    pub idempotency: Option<Arc<dyn IdempotencyStore>>,
    /// Endpoint deprecated do các module khai: `(tên module, khai báo)`.
    pub deprecations: Vec<(&'static str, Deprecation)>,
    /// Giới hạn thân request riêng của route do module khai: `(tên module, khai báo)`.
    pub body_limits: Vec<(&'static str, BodyLimit)>,
    /// Tool MCP của các module, tên đầy đủ (`Modules::tools`). Endpoint `/mcp` có mặt
    /// khi có `authenticator`.
    pub tools: Vec<crate::mcp::Tool>,
    /// Resource MCP của các module, URI đầy đủ (`Modules::mcp_resources`). Kernel thêm
    /// `autoxide://kernel/llms.txt` và `autoxide://kernel/openapi.json`.
    pub resources: Vec<crate::mcp::McpResource>,
    /// Prompt MCP của các module, tên đầy đủ (`Modules::prompts`).
    pub prompts: Vec<crate::mcp::Prompt>,
    /// Tên và mô tả của các module (`Modules::descriptions`), cho `/llms.txt`.
    pub modules: Vec<(&'static str, &'static str)>,
    /// URL công khai của API (`AUTOXIDE_HTTP_PUBLIC_URL`): link của `/llms.txt` tuyệt đối khi có.
    pub public_url: Option<String>,
    /// Bản dịch thông báo lỗi (mặc định: chỉ bản dịch của kernel thì dùng
    /// `Translations::new(Vec::new())`; rỗng thì mọi lỗi bằng tiếng Anh).
    pub translations: Translations,
    /// Sổ database của bản cài (bước 33): handler nhận
    /// [`TenantContext`](crate::tenancy::TenantContext) dùng nó. `None`: handler cần khách trả
    /// lỗi 500 nói rõ thiếu gì.
    pub tenancy: Option<crate::tenancy::DatabaseRegistry>,
    /// Quyền dùng theo khách (bước 35): route và tool MCP của module mà gói của khách không có bị
    /// chặn. Mặc định: không có bên cung cấp, không chặn gì.
    pub entitlements: crate::entitlement::EntitlementsHandle,
}

impl Extras {
    /// Sets the database registry (tenancy).
    pub fn tenancy(mut self, registry: crate::tenancy::DatabaseRegistry) -> Self {
        self.tenancy = Some(registry);
        self
    }

    /// Sets what each tenant may use (step 35).
    pub fn entitlements(mut self, entitlements: crate::entitlement::EntitlementsHandle) -> Self {
        self.entitlements = entitlements;
        self
    }

    /// Sets the authenticator a module published.
    pub fn authenticator(mut self, authenticator: Option<Arc<dyn Authenticator>>) -> Self {
        self.authenticator = authenticator;
        self
    }

    /// Sets the store for idempotency keys.
    pub fn idempotency(mut self, idempotency: Option<Arc<dyn IdempotencyStore>>) -> Self {
        self.idempotency = idempotency;
        self
    }

    /// Sets the deprecated endpoints: `(module name, declaration)`.
    pub fn deprecations(mut self, deprecations: Vec<(&'static str, Deprecation)>) -> Self {
        self.deprecations = deprecations;
        self
    }

    /// Sets the per-route request body limits: `(module name, declaration)`.
    pub fn body_limits(mut self, body_limits: Vec<(&'static str, BodyLimit)>) -> Self {
        self.body_limits = body_limits;
        self
    }

    /// Sets the MCP tools of the modules, with full names.
    pub fn tools(mut self, tools: Vec<crate::mcp::Tool>) -> Self {
        self.tools = tools;
        self
    }

    /// Sets the MCP resources of the modules, with full URIs.
    pub fn resources(mut self, resources: Vec<crate::mcp::McpResource>) -> Self {
        self.resources = resources;
        self
    }

    /// Sets the MCP prompts of the modules, with full names.
    pub fn prompts(mut self, prompts: Vec<crate::mcp::Prompt>) -> Self {
        self.prompts = prompts;
        self
    }

    /// Sets the names and one-line descriptions of the modules, for `/llms.txt`.
    pub fn modules(mut self, modules: Vec<(&'static str, &'static str)>) -> Self {
        self.modules = modules;
        self
    }

    /// Sets the public address of the API (`AUTOXIDE_HTTP_PUBLIC_URL`).
    pub fn public_url(mut self, public_url: Option<String>) -> Self {
        self.public_url = public_url;
        self
    }

    /// Sets the translations of error messages.
    pub fn translations(mut self, translations: Translations) -> Self {
        self.translations = translations;
        self
    }
}

pub fn build(
    config: &HttpConfig,
    root: OpenApiRouter,
    modules: Vec<(&'static str, OpenApiRouter)>,
    mut extras: Extras,
) -> Api {
    // `/llms.txt` và hai resource của kernel (bước 40 P2): dựng một lần từ khai báo của module.
    let mut resources = std::mem::take(&mut extras.resources);
    let openapi_slot: Arc<std::sync::OnceLock<String>> = Arc::default();
    let llms_slot: Arc<std::sync::OnceLock<String>> = Arc::default();
    resources.extend(kernel_resources(&openapi_slot, &llms_slot));
    // Module có đường dẫn: có tài liệu OpenAPI riêng (`/api/v1/openapi/<module>.json`).
    let documented: Vec<&'static str> = modules
        .iter()
        .filter(|(_, routes)| !routes.get_openapi().paths.paths.is_empty())
        .map(|(name, _)| *name)
        .collect();
    let llms_text = llms::render(&llms::LlmsInputs {
        base: extras.public_url.as_deref().unwrap_or_default(),
        api_docs: config.api_docs,
        mcp: extras.authenticator.is_some(),
        health: root.get_openapi().paths.paths.contains_key("/health/live"),
        catalog: modules.iter().any(|(name, routes)| {
            *name == crate::builtin::NAME
                && routes.get_openapi().paths.paths.contains_key("/modules/{name}")
        }),
        modules: &extras.modules,
        documented: &documented,
        tools: &extras.tools,
        resources: &resources,
        prompts: &extras.prompts,
    });
    let _ = llms_slot.set(llms_text.clone());
    let module_documents = openapi_modules::ModuleDocuments::default();
    let root = root
        .merge(llms::routes(llms_text))
        .merge(openapi_modules::routes(module_documents.clone()));

    let schema_conflicts = openapi::schema_conflicts(
        std::iter::once(("kernel", root.get_openapi()))
            .chain(modules.iter().map(|(name, routes)| (*name, routes.get_openapi()))),
    );
    let mut api = OpenApiRouter::new();
    for (name, mut routes) in modules {
        openapi::label_module(name, routes.get_openapi_mut());
        // Gói của khách (bước 35): đếm request ở mọi module; chặn ở module bị gói chi phối.
        if name != crate::user_flags::ME {
            let always = extras.entitlements.catalog().is_always_enabled(name);
            if !always {
                openapi::document_plan_gate(routes.get_openapi_mut());
            }
            let handle = extras.entitlements.clone();
            routes = routes.layer(middleware::from_fn(move |request: Request, next: Next| {
                entitlement::gate(handle.clone(), name, always, request, next)
            }));
        }
        // Lệnh cần duyệt khi agent gọi (bước 51 P12): `x-approval: "agent"` của module.
        let approvals = crate::ops::approvals::rules(name, routes.get_openapi_mut());
        if !approvals.is_empty() {
            let approvals = Arc::new(approvals);
            routes = routes.layer(middleware::from_fn(move |request: Request, next: Next| {
                crate::ops::approvals::gate(approvals.clone(), request, next)
            }));
        }
        api = api.nest(&format!("/{name}"), routes);
    }
    let (router, mut openapi) = OpenApiRouter::with_openapi(openapi::base_document())
        .merge(root)
        .nest(API_PREFIX, api)
        .split_for_parts();
    openapi::finish(&mut openapi, extras.idempotency.is_some());
    let deprecated = Arc::new(deprecation::apply(&mut openapi, &extras.deprecations));

    // Tài liệu không đổi trong lúc chạy: đổi thành JSON một lần rồi trả lại mãi. `Bytes` clone
    // chỉ tăng bộ đếm tham chiếu, không chép cả tài liệu (hơn 1 MB) mỗi request (D-158).
    let json = openapi.to_json().unwrap_or_default();
    let _ = openapi_slot.set(json.clone());
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) {
        let _ = module_documents.set(openapi_modules::split(&value, &documented));
    }
    let document = axum::body::Bytes::from(json);
    let mut router = router.route(
        OPENAPI_PATH,
        axum::routing::get(move || {
            let document = document.clone();
            async move { ([(header::CONTENT_TYPE, "application/json")], document) }
        }),
    );
    if config.api_docs {
        router = router.merge(Scalar::with_url(DOCS_PATH, openapi.clone()));
    }

    // REST và MCP xác thực qua bộ có giới hạn theo API key; lớp idempotency giữ bộ gốc (nó chỉ
    // xác thực để biết khoá thuộc về ai, không được đếm thêm một lần).
    let limited = extras.authenticator.clone().map(|inner| {
        // Đếm chung qua database (bước 40 P6) cần sổ database; không có thì đếm trong bộ nhớ.
        let shared = match config.rate_limit_store {
            crate::config::RateLimitStore::Database => extras.tenancy.clone(),
            _ => None,
        };
        rate_limit::ApiKeyLimited::wrap_in(inner, config.api_key_rate_limit, shared)
    });
    let translations = Arc::new(extras.translations);
    // Nhật ký request theo khách (bước 52): chỉ khi có sổ database và hạn lưu > 0.
    let request_log = extras.tenancy.as_ref().and_then(|registry| {
        crate::ops::request_log::RequestLog::start(registry.clone(), config.request_log_days)
    });
    let catalog = mcp::McpCatalog {
        tools: extras.tools,
        resources,
        prompts: std::mem::take(&mut extras.prompts),
    };
    if let Some(mcp) = mcp::router(
        catalog,
        translations.clone(),
        limited.clone(),
        &config.cors_origins,
        extras.entitlements.clone(),
        request_log.clone(),
    ) {
        router = router.merge(mcp);
    }

    let databases = extras.tenancy.clone();
    if let Some(registry) = extras.tenancy {
        // Sổ database trong "extensions" của mọi request, nơi extractor `TenantContext` tìm nó.
        router = router.layer(Extension(registry));
    }
    if let Some(authenticator) = limited {
        // Đặt bộ xác thực vào "extensions" của mọi request, nơi extractor
        // `Principal` tìm nó.
        router = router.layer(Extension(authenticator));
    }

    // `None` khi rate limiting bị tắt; lớp vẫn có mặt và cho mọi request đi qua.
    let body_limits = Arc::new(body_limit::BodyLimits::new(config.body_limit, &extras.body_limits));
    let idempotency = extras.idempotency.map(|store| {
        Arc::new(idempotency::Settings {
            store,
            authenticator: extras.authenticator.clone(),
            body_limits: body_limits.clone(),
            request_timeout: config.request_timeout,
            databases: databases.clone(),
        })
    });
    let limiter = rate_limit::RateLimiter::new(config.rate_limit).map(Arc::new);
    let client_address = Arc::new(client::ClientAddress::new(
        config.client_ip_header.clone(),
        config.trusted_proxy_count,
        config.trusted_proxies.clone(),
    ));
    let request_timeout = config.request_timeout;
    let security = security_headers::Prepared::new(&config.security_headers);
    // `.layer` bọc thêm một lớp RA NGOÀI các lớp trước đó: lớp khai sau cùng là lớp
    // request gặp đầu tiên. Đọc từ dưới lên sẽ ra thứ tự ở đầu file.
    let router = router
        .layer(middleware::from_fn(conditional::not_modified))
        // Khách có thể chọn bằng header `autoxide-tenant` (bước 33): cache dùng chung đứng trước API
        // (CDN, reverse proxy) phải tách bản lưu theo header này, không thì trả nội dung của khách
        // này cho khách khác. Đặt ngoài lớp 304 để cả 304 cũng mang nó.
        .layer(middleware::map_response(vary_tenant))
        // Giới hạn mặc định của axum (2 MiB cho extractor) tắt đi: lớp `body_limit`
        // đã giới hạn theo từng route.
        .layer(DefaultBodyLimit::disable())
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            body_limit::limit(body_limits.clone(), request, next)
        }))
        .layer(middleware::from_fn(refuse_nul_in_url))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            enforce_timeout(request_timeout, request, next)
        }))
        .layer(CatchPanicLayer::custom(problem::PanicToProblem))
        .layer(middleware::map_response(problem::normalize))
        // NGOÀI lớp chuẩn hoá: mọi lỗi đã là Problem kèm `Error`, sẵn để dịch.
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            language::localize(translations.clone(), request, next)
        }))
        // NGOÀI lớp ngôn ngữ: lưu đúng response client nhận lần đầu.
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            idempotency::idempotency(idempotency.clone(), request, next)
        }))
        // TRONG lớp CORS: request preflight của trình duyệt không bị đếm, còn response
        // 429 vẫn mang header CORS để trang web đọc được lỗi.
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            rate_limit::limit(limiter.clone(), request, next)
        }))
        .layer(cors(&config.cors_origins))
        // NGOÀI rate limiting: giải địa chỉ client một lần, mọi lớp sau và handler dùng chung.
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            client::attach(client_address.clone(), request, next)
        }))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            deprecation::mark(deprecated.clone(), request, next)
        }))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            log::log_requests(request_log.clone(), request, next)
        }))
        // NGOÀI lớp log, trong lớp request id: mọi response (404, 429, preflight…) đều có header.
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            security_headers::add(security.clone(), request, next)
        }))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, request_id::MakeUuidV7))
        .layer(middleware::from_fn(request_id::drop_untrusted));

    Api { router, openapi, schema_conflicts }
}

/// Hai resource của kernel: `/llms.txt` và tài liệu OpenAPI, ai đăng nhập cũng đọc được (cùng nội
/// dung như hai trang công khai). Nội dung điền vào ô sau khi dựng xong.
fn kernel_resources(
    openapi: &Arc<std::sync::OnceLock<String>>,
    llms: &Arc<std::sync::OnceLock<String>>,
) -> Vec<crate::mcp::McpResource> {
    let read = |slot: &Arc<std::sync::OnceLock<String>>| {
        let slot = slot.clone();
        move |_: crate::tenancy::TenantContext, _: crate::access::Principal| {
            let text = slot.get().cloned();
            async move { text.ok_or_else(|| Error::internal("the document is not built yet")) }
        }
    };
    vec![
        crate::mcp::McpResource::new(
            "llms.txt",
            "llms.txt",
            "Overview of this server for agents: where the REST and MCP contracts are, how to \
             authenticate, the conventions, the installed modules, prompts and resources. The \
             same text as `/llms.txt`.",
            "text/markdown",
            read(llms),
        )
        .for_any_signed_in()
        .qualify(crate::builtin::NAME),
        crate::mcp::McpResource::new(
            "openapi.json",
            "OpenAPI document",
            "The full REST contract (OpenAPI 3.1): every endpoint, its parameters, responses, \
             error codes and the permission it needs. The same document as \
             `/api/v1/openapi.json`.",
            "application/json",
            read(openapi),
        )
        .for_any_signed_in()
        .qualify(crate::builtin::NAME),
    ]
}

/// Từ chối request có ký tự NUL (`%00`) trong đường dẫn hay query: giá trị đó tới database thì
/// Postgres lỗi 500, SQLite thì không (D-157). Một lớp cho mọi route, thay vì từng extractor.
async fn refuse_nul_in_url(request: Request, next: Next) -> Response {
    let target = request.uri().path_and_query().map(|target| target.as_str()).unwrap_or_default();
    if target.contains("%00") {
        return nul_refused().into_response();
    }
    next.run(request).await
}

async fn enforce_timeout(limit: Duration, request: Request, next: Next) -> Response {
    match tokio::time::timeout(limit, next.run(request)).await {
        Ok(response) => response,
        Err(_) => {
            let mut response = timed_out(limit).into_response();
            response.extensions_mut().insert(idempotency::OutcomeUnknown);
            response
        }
    }
}

/// Lỗi khi request chạy quá `limit`.
fn timed_out(limit: Duration) -> Error {
    Error::unavailable(
        "request_timeout",
        "The server did not finish handling the request within {seconds} seconds. Retry later.",
    )
    .with_param("seconds", limit.as_secs())
}

/// Thêm `autoxide-tenant` vào `Vary` của response (giữ các giá trị đã có).
async fn vary_tenant(mut response: Response) -> Response {
    response.headers_mut().append(header::VARY, HeaderValue::from_static("autoxide-tenant"));
    response
}

fn cors(origins: &CorsOrigins) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::ACCEPT,
            header::ACCEPT_LANGUAGE,
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::IF_MATCH,
            header::IF_NONE_MATCH,
            header::IF_RANGE,
            header::RANGE,
            idempotency::IDEMPOTENCY_KEY,
            REQUEST_ID_HEADER,
            crate::tenancy::TENANT_HEADER,
            // Header của giao thức MCP (endpoint `/mcp`).
            HeaderName::from_static("mcp-protocol-version"),
            HeaderName::from_static("mcp-session-id"),
            HeaderName::from_static("last-event-id"),
        ])
        // Header trình duyệt mặc định giấu khỏi JavaScript mà client cần đọc.
        .expose_headers([
            header::ETAG,
            header::LOCATION,
            header::CONTENT_DISPOSITION,
            header::CONTENT_RANGE,
            header::ACCEPT_RANGES,
            header::RETRY_AFTER,
            header::LINK,
            HeaderName::from_static("deprecation"),
            HeaderName::from_static("sunset"),
            REQUEST_ID_HEADER,
            idempotency::IDEMPOTENT_REPLAYED,
            HeaderName::from_static("mcp-session-id"),
        ])
        .max_age(Duration::from_secs(3600));
    match origins {
        // Không khai origin nào: không thêm header CORS, trình duyệt tự chặn.
        CorsOrigins::None => layer,
        CorsOrigins::Any => layer.allow_origin(AllowOrigin::any()),
        CorsOrigins::List(list) => layer.allow_origin(AllowOrigin::list(
            // Cấu hình đã kiểm origin là ASCII hợp lệ; `filter_map` chỉ để khỏi `unwrap`.
            list.iter().filter_map(|origin| HeaderValue::from_str(origin).ok()),
        )),
    }
}
