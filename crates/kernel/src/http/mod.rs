//! Máy chủ HTTP: ghép route của các module thành một router và bọc quanh nó các
//! lớp xử lý chung cho mọi request.
//!
//! Route của module `notes` nằm dưới `/api/v1/notes`. Số `v1` là phiên bản của
//! API: thay đổi phá vỡ thì mở `/api/v2`, không sửa `v1`.
//!
//! Mọi request đi qua, theo thứ tự từ ngoài vào:
//!
//! 1. **request id** — mỗi request một ID (header `x-request-id`), trả lại trong
//!    response; client gửi sẵn ID hợp lệ thì dùng ID đó;
//! 2. **CORS** — trình duyệt ở origin nào được gọi;
//! 3. **chuẩn hoá lỗi** — mọi response lỗi đều là Problem Details (RFC 9457), kể
//!    cả lỗi do thư viện sinh ra (404, 405, 413…);
//! 4. **bắt panic** — handler panic thành lỗi 500 thay vì đứt kết nối;
//! 5. **timeout** — request chạy quá lâu bị cắt, trả 503;
//! 6. **giới hạn body** — body quá lớn bị từ chối, trả 413.

mod extract;
mod problem;
mod request_id;
mod server;

use std::time::Duration;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderName, HeaderValue, Method, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{PropagateRequestIdLayer, SetRequestIdLayer};
use utoipa::openapi::{InfoBuilder, OpenApi, OpenApiBuilder};
use utoipa_axum::router::OpenApiRouter;

use crate::config::{CorsOrigins, HttpConfig};
use crate::error::Error;

pub use extract::ValidJson;
pub use request_id::REQUEST_ID_HEADER;
pub use server::{Shutdown, serve, shutdown_signal};

/// Tiền tố của mọi route API.
pub const API_PREFIX: &str = "/api/v1";

/// Router đã ghép xong, kèm tài liệu OpenAPI của mọi route trong nó.
pub struct Api {
    pub router: Router,
    pub openapi: OpenApi,
}

/// Ghép route của các module (tên module, router) dưới [`API_PREFIX`] và bọc các
/// lớp xử lý chung.
pub fn build(config: &HttpConfig, modules: Vec<(&'static str, OpenApiRouter)>) -> Api {
    let mut api = OpenApiRouter::new();
    for (name, routes) in modules {
        api = api.nest(&format!("/{name}"), routes);
    }
    let (router, openapi) =
        OpenApiRouter::with_openapi(base_document()).nest(API_PREFIX, api).split_for_parts();

    let request_timeout = config.request_timeout;
    // `.layer` bọc thêm một lớp RA NGOÀI các lớp trước đó: lớp khai sau cùng là lớp
    // request gặp đầu tiên. Đọc từ dưới lên sẽ ra thứ tự ở đầu file.
    let router = router
        .layer(DefaultBodyLimit::max(config.body_limit))
        .layer(RequestBodyLimitLayer::new(config.body_limit))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            enforce_timeout(request_timeout, request, next)
        }))
        .layer(CatchPanicLayer::custom(problem::PanicToProblem))
        .layer(middleware::map_response(problem::normalize))
        .layer(cors(&config.cors_origins))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER))
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER, request_id::MakeUuidV7))
        .layer(middleware::from_fn(request_id::drop_untrusted));

    Api { router, openapi }
}

/// Phần đầu của tài liệu OpenAPI; mục "OpenAPI" của lộ trình bổ sung thêm.
fn base_document() -> OpenApi {
    OpenApiBuilder::new()
        .info(InfoBuilder::new().title("Autoxide API").version(env!("CARGO_PKG_VERSION")).build())
        .build()
}

async fn enforce_timeout(limit: Duration, request: Request, next: Next) -> Response {
    match tokio::time::timeout(limit, next.run(request)).await {
        Ok(response) => response,
        Err(_) => Error::unavailable(
            "request_timeout",
            "The server did not finish handling the request within {seconds} seconds. Retry later.",
        )
        .with_param("seconds", limit.as_secs())
        .into_response(),
    }
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
            HeaderName::from_static("idempotency-key"),
            REQUEST_ID_HEADER,
        ])
        .expose_headers([REQUEST_ID_HEADER])
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
