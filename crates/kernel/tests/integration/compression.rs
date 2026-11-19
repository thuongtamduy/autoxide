//! Nén response (bước 39 B4, D-20261119-step39-compression): lớp HTTP của kernel nén JSON và chữ
//! khi client nhận, không nén file đã nén / nhị phân, response một khúc và stream sự kiện, và `ETag`
//! vẫn cho `304`. Không chạm database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, HttpConfig};
use autoxide_kernel::http::{self, Api};
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::response::Response;
use tower::ServiceExt;
use utoipa_axum::router::OpenApiRouter;

/// JSON ~20 KB, nén tốt.
fn big_json() -> String {
    let items: Vec<String> =
        (0..400).map(|i| format!("{{\"id\":{i},\"title\":\"a fairly long title {i}\"}}")).collect();
    format!("{{\"data\":[{}]}}", items.join(","))
}

fn with_type(content_type: &'static str, body: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

async fn json() -> Response {
    let mut response = with_type("application/json", big_json().into_bytes());
    response.headers_mut().insert(header::ETAG, HeaderValue::from_static("\"v1\""));
    response
}

async fn small() -> Response {
    with_type("application/json", b"{\"data\":1}".to_vec())
}

async fn image() -> Response {
    with_type("image/jpeg", vec![b'a'; 20_000])
}

async fn events() -> Response {
    with_type("text/event-stream", big_json().into_bytes())
}

async fn part() -> Response {
    let mut response = with_type("application/json", big_json().into_bytes());
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    response
        .headers_mut()
        .insert(header::CONTENT_RANGE, HeaderValue::from_static("bytes 0-99/200"));
    response
}

fn app(compression: bool) -> Router {
    let value = if compression { "true" } else { "false" };
    let source = ConfigSource::from_pairs([("AUTOXIDE_HTTP_COMPRESSION", value)]);
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let config: HttpConfig = reader.finish(config).unwrap().0;
    let routes = OpenApiRouter::new()
        .route("/json", axum::routing::get(json))
        .route("/small", axum::routing::get(small))
        .route("/image", axum::routing::get(image))
        .route("/events", axum::routing::get(events))
        .route("/part", axum::routing::get(part));
    let Api { router, .. } =
        http::build(&config, OpenApiRouter::new(), vec![("x", routes)], http::Extras::default());
    router
}

async fn call(app: &Router, path: &str, encoding: &str) -> (Response, Vec<u8>) {
    let request = Request::builder()
        .uri(format!("/api/v1/x{path}"))
        .header(header::ACCEPT_ENCODING, encoding)
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap().to_vec();
    (Response::from_parts(parts, Body::empty()), bytes)
}

fn encoding(response: &Response) -> Option<&str> {
    response.headers().get(header::CONTENT_ENCODING).map(|value| value.to_str().unwrap())
}

#[tokio::test]
async fn nen_json_lon_khi_client_nhan_br() {
    let app = app(true);
    let plain = big_json().len();
    let (response, body) = call(&app, "/json", "br").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(encoding(&response), Some("br"));
    assert!(body.len() * 4 < plain, "{} of {plain}", body.len());
    let vary: Vec<_> = response.headers().get_all(header::VARY).iter().collect();
    assert!(
        vary.iter().any(|value| value.to_str().unwrap().contains("accept-encoding")),
        "{vary:?}"
    );
    let (response, body) = call(&app, "/json", "gzip").await;
    assert_eq!(encoding(&response), Some("gzip"));
    assert_eq!(&body[..2], &[0x1f, 0x8b], "gzip magic");
    // Client không xin nén: nguyên văn.
    let (response, body) = call(&app, "/json", "identity").await;
    assert_eq!(encoding(&response), None);
    assert_eq!(body.len(), plain);
}

#[tokio::test]
async fn khong_nen_anh_va_range() {
    let app = app(true);
    for path in ["/image", "/part", "/events", "/small"] {
        let (response, _) = call(&app, path, "br, gzip").await;
        assert_eq!(encoding(&response), None, "{path}");
    }
}

#[tokio::test]
async fn etag_van_304_sau_khi_nen() {
    let app = app(true);
    let (response, _) = call(&app, "/json", "br").await;
    assert_eq!(response.headers()[header::ETAG], "\"v1\"");
    let request = Request::builder()
        .uri("/api/v1/x/json")
        .header(header::ACCEPT_ENCODING, "br")
        .header(header::IF_NONE_MATCH, "\"v1\"")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()[header::ETAG], "\"v1\"");
}

#[tokio::test]
async fn tat_bang_cau_hinh_thi_khong_nen() {
    let (response, body) = call(&app(false), "/json", "br, gzip").await;
    assert_eq!(encoding(&response), None);
    assert_eq!(body.len(), big_json().len());
}
