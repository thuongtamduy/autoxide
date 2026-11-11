//! Metrics của lớp HTTP. Bộ thu số đo là toàn tiến trình và chỉ bật được một lần,
//! nên file này (một tiến trình test riêng) chỉ có MỘT bài test.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_kernel::config::{ConfigReader, ConfigSource, HttpConfig};
use autoxide_kernel::error::Error;
use autoxide_kernel::http;
use autoxide_kernel::metrics::Metrics;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Get one note.
#[utoipa::path(get, path = "/{id}", responses((status = 200, description = "The note.")))]
async fn get_note(Path(id): Path<u32>) -> Result<String, Error> {
    if id == 404 {
        return Err(Error::not_found("notes.note_not_found", "No such note."));
    }
    Ok(format!("note {id}"))
}

#[tokio::test]
async fn so_do_cua_request_duoc_thu_va_phuc_vu_o_metrics() {
    // Trước khi bật: ghi số đo không làm gì và không lỗi.
    metrics::counter!("ignored_total").increment(1);

    let metrics = Metrics::install().unwrap();
    metrics.spawn_upkeep();
    // Bật lần hai là lỗi, không phải lặng lẽ thay bộ thu cũ.
    assert!(Metrics::install().is_err());

    let source = ConfigSource::default();
    let mut reader = ConfigReader::new(&source);
    let config = HttpConfig::read(&mut reader);
    let config = reader.finish(config).unwrap().0;
    let notes = OpenApiRouter::new().routes(routes!(get_note));
    let api = http::build(&config, http::metrics_routes(metrics), vec![("notes", notes)]);
    let send = async |path: &str| {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        api.router.clone().oneshot(request).await.unwrap()
    };

    // Ba request tới CÙNG một mẫu route với ba ID khác nhau, một request lỗi, một
    // request không khớp route nào.
    for path in ["/api/v1/notes/1", "/api/v1/notes/2", "/api/v1/notes/3"] {
        assert_eq!(send(path).await.status(), StatusCode::OK);
    }
    assert_eq!(send("/api/v1/notes/404").await.status(), StatusCode::NOT_FOUND);
    assert_eq!(send("/some/random/path").await.status(), StatusCode::NOT_FOUND);

    let response = send("/metrics").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/plain; version=0.0.4; charset=utf-8"
    );
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();

    // Nhãn `route` là mẫu route: ba ID khác nhau gộp vào một chuỗi số đo.
    let line = |needle: &str| {
        text.lines()
            .find(|line| line.starts_with(needle))
            .unwrap_or_else(|| panic!("{needle}\n{text}"))
    };
    assert!(
        line(r#"http_requests_total{method="GET",route="/api/v1/notes/{id}",status="200"}"#)
            .ends_with(" 3")
    );
    assert!(
        line(r#"http_requests_total{method="GET",route="/api/v1/notes/{id}",status="404"}"#)
            .ends_with(" 1")
    );
    // Đường dẫn không khớp route nào dùng chung nhãn `unmatched`, không lộ đường dẫn thô.
    assert!(
        line(r#"http_requests_total{method="GET",route="unmatched",status="404"}"#).ends_with(" 1")
    );
    assert!(!text.contains("/some/random/path"), "{text}");
    assert!(!text.contains("ignored_total"), "{text}");

    // Thời gian xử lý là histogram có các xô đã định.
    assert!(
        line(r#"http_request_duration_seconds_count{method="GET",route="/api/v1/notes/{id}"}"#)
            .ends_with(" 4")
    );
    assert!(text.contains(r#"http_request_duration_seconds_bucket{method="GET",route="/api/v1/notes/{id}",le="0.005"}"#));

    // `/metrics` có trong tài liệu OpenAPI và đủ mô tả.
    assert!(api.openapi.paths.paths.contains_key("/metrics"));
    assert_eq!(http::lint(&api.openapi), Vec::<String>::new());
}
