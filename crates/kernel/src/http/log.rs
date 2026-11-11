//! Ghi log và số đo cho từng request: một dòng log khi request xong, kèm request id.
//!
//! Mọi log phát ra TRONG lúc xử lý request (từ handler, service) tự mang request id
//! nhờ span `request` — tra một request id là ra toàn bộ dấu vết của request đó.
//!
//! Chỉ ghi đường dẫn, KHÔNG ghi query string hay thân request: chúng có thể chứa
//! token hoặc dữ liệu cá nhân.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::request_id::REQUEST_ID_HEADER;
use crate::error::Error;

pub(super) async fn log_requests(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    // Mẫu route đã khớp (vd `/api/v1/notes/{id}`); không có khi request không khớp
    // route nào.
    let route =
        request.extensions().get::<MatchedPath>().map(|matched| matched.as_str().to_owned());
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();

    let span = tracing::info_span!(
        "request",
        request_id,
        %method,
        path,
        // Ba trường dưới dành cho trace OpenTelemetry (tên theo quy ước của nó);
        // hai trường sau chỉ biết khi request đã xong nên để trống trước.
        "otel.kind" = "server",
        "http.route" = tracing::field::Empty,
        "http.response.status_code" = tracing::field::Empty,
    );
    // Request mang `traceparent` thì span này nối vào trace của bên gọi. Không bật
    // xuất trace thì lời gọi này không có tác dụng.
    let _ = span.set_parent(crate::telemetry::remote_context(request.headers()));
    let started = Instant::now();
    // `instrument`: mọi log phát ra trong lúc chạy phần còn lại đều thuộc span này.
    let response = next.run(request).instrument(span.clone()).await;

    let status = response.status().as_u16();
    let elapsed = started.elapsed();
    let elapsed_ms = elapsed.as_millis() as u64;
    let route = route.as_deref().unwrap_or("unmatched");
    super::metrics::record_request(method.as_str(), route, status, elapsed.as_secs_f64());
    span.record("http.route", route);
    span.record("http.response.status_code", status);
    span.in_scope(|| match response.extensions().get::<Arc<Error>>() {
        // Lỗi phía máy chủ: ghi cả nguyên nhân gốc — thứ client không được thấy.
        Some(error) if status >= 500 => {
            tracing::error!(status, elapsed_ms, route, error = %error, "request failed");
        }
        _ if status >= 500 => tracing::error!(status, elapsed_ms, route, "request failed"),
        // Health check bị gọi vài giây một lần: ghi ở mức debug để không lấp log.
        _ if path.starts_with("/health/") => {
            tracing::debug!(status, elapsed_ms, route, "request completed");
        }
        _ => tracing::info!(status, elapsed_ms, route, "request completed"),
    });
    response
}
