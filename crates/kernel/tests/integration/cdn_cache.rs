//! `Cache-Control`, `ETag`, `Vary` cho CDN (bước 39 B5, D-20261119-step39-cdn-cache): đọc công khai
//! được CDN giữ `s-maxage` giây, có ETag và `304`; request đăng nhập không bao giờ nhận `public`;
//! file có phiên bản giữ `immutable`; bản xem trước giữ `no-store`. Không chạm database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, HttpConfig};
use autoxide_kernel::http::{self, Api};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::response::Response;
use tower::ServiceExt;
use utoipa_axum::router::OpenApiRouter;

fn with_cache(value: &'static str, body: &'static str) -> Response {
    let mut response = Response::new(Body::from(body));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    response
}

async fn public() -> Response {
    with_cache("public, no-cache", "{\"data\":[1,2,3]}")
}

async fn media() -> Response {
    with_cache("public, max-age=31536000, immutable", "{\"data\":1}")
}

async fn preview() -> Response {
    with_cache("no-store", "{\"data\":\"draft\"}")
}

async fn plain() -> &'static str {
    "ok"
}

fn app(seconds: &str) -> Router {
    let source = ConfigSource::from_pairs([("AUTOXIDE_HTTP_PUBLIC_CACHE_SECONDS", seconds)]);
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let config: HttpConfig = reader.finish(config).unwrap().0;
    let routes = OpenApiRouter::new()
        .route("/public", axum::routing::get(public))
        .route("/media", axum::routing::get(media))
        .route("/preview", axum::routing::get(preview))
        .route("/plain", axum::routing::get(plain).post(plain));
    let Api { router, .. } =
        http::build(&config, OpenApiRouter::new(), vec![("x", routes)], http::Extras::default());
    router
}

async fn call(app: &Router, method: &str, path: &str, headers: &[(&str, &str)]) -> Response {
    let mut request = Request::builder().method(method).uri(format!("/api/v1/x{path}"));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap()
}

fn cache_control(response: &Response) -> &str {
    response.headers()[header::CACHE_CONTROL].to_str().unwrap()
}

fn vary(response: &Response) -> String {
    let values: Vec<_> =
        response.headers().get_all(header::VARY).iter().map(|v| v.to_str().unwrap()).collect();
    values.join(", ").to_ascii_lowercase()
}

#[tokio::test]
async fn endpoint_cong_khai_co_s_maxage_etag_vary() {
    let app = app("60");
    let response = call(&app, "GET", "/public", &[]).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        cache_control(&response),
        "public, max-age=0, s-maxage=60, stale-while-revalidate=300"
    );
    let vary = vary(&response);
    for name in ["accept-language", "accept-encoding", "origin", "autoxide-tenant"] {
        assert!(vary.contains(name), "{name} in {vary}");
    }
    let etag = response.headers()[header::ETAG].to_str().unwrap().to_owned();
    assert!(etag.starts_with("W/\""), "{etag}");
    // Cùng nội dung → cùng ETag; hỏi lại bằng ETag → 304.
    let again = call(&app, "GET", "/public", &[]).await;
    assert_eq!(again.headers()[header::ETAG], etag.as_str());
    let not_modified = call(&app, "GET", "/public", &[("if-none-match", &etag)]).await;
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
    // `0`: CDN cũng phải hỏi lại.
    let response = call(&self::app("0"), "GET", "/public", &[]).await;
    assert_eq!(cache_control(&response), "public, no-cache");
}

#[tokio::test]
async fn response_co_authorization_khong_bao_gio_public() {
    let app = app("60");
    for header in [("authorization", "Bearer abc"), ("cookie", "session=1")] {
        let response = call(&app, "GET", "/public", &[header]).await;
        assert_eq!(cache_control(&response), "private, no-cache", "{header:?}");
        let response = call(&app, "GET", "/media", &[header]).await;
        assert_eq!(cache_control(&response), "private, max-age=31536000, immutable");
        let response = call(&app, "GET", "/plain", &[header]).await;
        assert_eq!(cache_control(&response), "private, no-store");
    }
    // Ghi: `no-store`.
    let response = call(&app, "POST", "/plain", &[]).await;
    assert_eq!(cache_control(&response), "no-store");
}

#[tokio::test]
async fn media_co_phien_ban_immutable() {
    let response = call(&app("60"), "GET", "/media", &[]).await;
    assert_eq!(cache_control(&response), "public, max-age=31536000, immutable");
    assert!(!response.headers().contains_key(header::ETAG));
}

#[tokio::test]
async fn preview_no_store() {
    let app = app("60");
    let response = call(&app, "GET", "/preview", &[]).await;
    assert_eq!(cache_control(&response), "no-store");
    let response = call(&app, "GET", "/preview", &[("authorization", "Bearer abc")]).await;
    assert_eq!(cache_control(&response), "no-store");
}
