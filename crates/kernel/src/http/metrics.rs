//! Endpoint `/metrics` cho Prometheus, và số đo của chính lớp HTTP.

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::metrics::Metrics;

/// Kiểu nội dung của định dạng văn bản Prometheus.
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Route `/metrics`, gắn ở gốc (ngoài `/api/v1`).
pub fn metrics_routes(metrics: Metrics) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(scrape)).with_state(metrics)
}

/// Metrics in the Prometheus text format.
///
/// Includes `http_requests_total` (by method, route and status) and
/// `http_request_duration_seconds`. Served only when enabled in the configuration.
#[utoipa::path(
    get,
    path = "/metrics",
    // Không đăng nhập: máy thu số đo gọi; chỉ có khi bật `AUTOXIDE_HTTP_METRICS` (D-245).
    extensions(("x-public" = json!(true))),
    tag = "health",
    operation_id = "metrics",
    responses((status = 200, description = "Current metrics.", body = String, content_type = "text/plain"))
)]
async fn scrape(State(metrics): State<Metrics>) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)], metrics.render())
}

/// Ghi số đo cho một request đã xong. `route` là MẪU route (vd
/// `/api/v1/notes/{id}`), không phải đường dẫn thô — để số chuỗi số đo không tăng
/// theo số ID.
pub(super) fn record_request(method: &str, route: &str, status: u16, elapsed_seconds: f64) {
    // HTTP nhận method bất kỳ (`FOO`, `M123`…): nhãn chỉ lấy trong tập cố định, không thì mỗi
    // method lạ thêm một chuỗi số đo sống mãi trong bộ nhớ (D-157).
    let method = match method {
        "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" => method,
        _ => "other",
    };
    let labels = [
        ("method", method.to_owned()),
        ("route", route.to_owned()),
        ("status", status.to_string()),
    ];
    metrics::counter!("http_requests_total", &labels).increment(1);
    metrics::histogram!("http_request_duration_seconds", &labels[..2]).record(elapsed_seconds);
}
