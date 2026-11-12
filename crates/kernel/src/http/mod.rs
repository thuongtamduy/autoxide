//! Máy chủ HTTP: ghép route của các module thành một router và bọc quanh nó các
//! lớp xử lý chung cho mọi request.
//!
//! Route của module `notes` nằm dưới `/api/v1/notes`. Số `v1` là phiên bản của
//! API: thay đổi phá vỡ thì mở `/api/v2`, không sửa `v1`.
//!
//! Tài liệu OpenAPI của mọi route nằm ở `/api/v1/openapi.json`; trang xem tài liệu
//! ở `/api/docs` khi được bật bằng cấu hình.
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

mod body_limit;
mod client;
mod conditional;
mod deprecation;
mod disposition;
mod extract;
mod health;
mod idempotency;
mod language;
mod log;
mod mcp;
mod metrics;
mod openapi;
mod problem;
mod rate_limit;
mod request_id;
mod server;

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
pub use deprecation::Deprecation;
pub use disposition::{Disposition, content_disposition};
pub use extract::ValidJson;
pub use health::{Health, health_routes};
pub use mcp::MCP_PATH;
pub use metrics::metrics_routes;
pub(crate) use openapi::untranslated_text;
pub use openapi::{BEARER_AUTH, DOCS_PATH, OPENAPI_PATH, lint};
pub use request_id::REQUEST_ID_HEADER;
pub use server::{Shutdown, serve, shutdown_signal};

/// Tiền tố của mọi route API.
pub const API_PREFIX: &str = "/api/v1";

/// Router đã ghép xong, kèm tài liệu OpenAPI của mọi route trong nó.
pub struct Api {
    pub router: Router,
    pub openapi: OpenApi,
}

/// Ghép route của các module (tên module, router) dưới [`API_PREFIX`], thêm các
/// route `root` ở gốc (ngoài `/api/v1`, vd [`health_routes`]), và bọc các lớp xử lý
/// chung quanh tất cả.
///
/// Những thứ dùng chung cho mọi request, ngoài route.
#[derive(Clone, Default)]
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
    /// Bản dịch thông báo lỗi (mặc định: chỉ bản dịch của kernel thì dùng
    /// `Translations::new(Vec::new())`; rỗng thì mọi lỗi bằng tiếng Anh).
    pub translations: Translations,
}

pub fn build(
    config: &HttpConfig,
    root: OpenApiRouter,
    modules: Vec<(&'static str, OpenApiRouter)>,
    extras: Extras,
) -> Api {
    let mut api = OpenApiRouter::new();
    for (name, mut routes) in modules {
        openapi::label_module(name, routes.get_openapi_mut());
        api = api.nest(&format!("/{name}"), routes);
    }
    let (router, mut openapi) = OpenApiRouter::with_openapi(openapi::base_document())
        .merge(root)
        .nest(API_PREFIX, api)
        .split_for_parts();
    openapi::finish(&mut openapi, extras.idempotency.is_some());
    let deprecated = Arc::new(deprecation::apply(&mut openapi, &extras.deprecations));

    // Tài liệu không đổi trong lúc chạy: đổi thành JSON một lần rồi trả lại mãi.
    let document = openapi.to_json().unwrap_or_default();
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

    let translations = Arc::new(extras.translations);
    if let Some(mcp) = mcp::router(
        extras.tools,
        translations.clone(),
        extras.authenticator.clone(),
        &config.cors_origins,
    ) {
        router = router.merge(mcp);
    }

    if let Some(authenticator) = extras.authenticator.clone() {
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
        })
    });
    let limiter = rate_limit::RateLimiter::new(config.rate_limit).map(Arc::new);
    let client_address = Arc::new(client::ClientAddress::new(
        config.client_ip_header.clone(),
        config.trusted_proxy_count,
    ));
    let request_timeout = config.request_timeout;
    // `.layer` bọc thêm một lớp RA NGOÀI các lớp trước đó: lớp khai sau cùng là lớp
    // request gặp đầu tiên. Đọc từ dưới lên sẽ ra thứ tự ở đầu file.
    let router = router
        .layer(middleware::from_fn(conditional::not_modified))
        // Giới hạn mặc định của axum (2 MiB cho extractor) tắt đi: lớp `body_limit`
        // đã giới hạn theo từng route.
        .layer(DefaultBodyLimit::disable())
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            body_limit::limit(body_limits.clone(), request, next)
        }))
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
        .layer(middleware::from_fn(log::log_requests))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, request_id::MakeUuidV7))
        .layer(middleware::from_fn(request_id::drop_untrusted));

    Api { router, openapi }
}

async fn enforce_timeout(limit: Duration, request: Request, next: Next) -> Response {
    match tokio::time::timeout(limit, next.run(request)).await {
        Ok(response) => response,
        Err(_) => timed_out(limit).into_response(),
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
