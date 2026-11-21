//! Ghi log và số đo cho từng request: một dòng log khi request xong, kèm request id.
//!
//! Mọi log phát ra TRONG lúc xử lý request (từ handler, service) tự mang request id
//! nhờ span `request` — tra một request id là ra toàn bộ dấu vết của request đó.
//!
//! Chỉ ghi MẪU route đã khớp (vd `/api/v1/notes/{id}`), KHÔNG ghi đường dẫn thô, query string hay
//! thân request: chúng có thể chứa token (link tải file số, trạng thái thanh toán, webhook của đối
//! tác có khoá trong đường dẫn) hoặc dữ liệu cá nhân. Request không khớp route nào ghi `unmatched`.

use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::request_id::REQUEST_ID_HEADER;
use crate::error::Error;
use crate::ops::request_log::{CallerSlot, Finished, RequestLog, header_text};
use opentelemetry::trace::TraceContextExt;

/// Giá trị `http.route` của request không khớp route nào.
const UNMATCHED: &str = "unmatched";

pub(super) async fn log_requests(
    request_log: Option<RequestLog>,
    mut request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    // Ô người gọi: extractor `Principal` / `TenantContext` điền khi giải ra (bước 52).
    let caller = CallerSlot::default();
    if request_log.is_some() {
        request.extensions_mut().insert(caller.clone());
    }
    let idempotency_key = header_text(request.headers().get("idempotency-key"), 255);
    let approval_header = request
        .headers()
        .get(&crate::ops::approvals::APPROVAL_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<uuid::Uuid>().ok());
    // Mẫu route đã khớp (vd `/api/v1/notes/{id}`); không có khi request không khớp
    // route nào. Đường dẫn thô không bao giờ vào span, log hay số đo (xem đầu file).
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or_else(|| UNMATCHED.to_owned(), |matched| matched.as_str().to_owned());
    let health = request.uri().path().starts_with("/health/");
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
        "http.route" = route.as_str(),
        // `http.route` ở trên và hai trường dưới dành cho trace OpenTelemetry (tên theo quy ước
        // của nó); mã trạng thái chỉ biết khi request đã xong nên để trống trước.
        "otel.kind" = "server",
        "http.response.status_code" = tracing::field::Empty,
        // Khách của request (bước 39 B7, D-190): biết khi extractor tìm ra khách / người gọi, ghi
        // qua `RequestSpan`. Để tra một khách bằng log và trace — KHÔNG làm nhãn số đo.
        tenant_id = tracing::field::Empty,
    );
    request.extensions_mut().insert(RequestSpan(span.clone()));
    // Request mang `traceparent` thì span này nối vào trace của bên gọi. Không bật
    // xuất trace thì lời gọi này không có tác dụng.
    let _ = span.set_parent(crate::telemetry::remote_context(request.headers()));
    let started = Instant::now();
    // `instrument`: mọi log phát ra trong lúc chạy phần còn lại đều thuộc span này.
    let response = next.run(request).instrument(span.clone()).await;

    let status = response.status().as_u16();
    let elapsed = started.elapsed();
    let elapsed_ms = elapsed.as_millis() as u64;
    let route = route.as_str();
    super::metrics::record_request(method.as_str(), route, status, elapsed.as_secs_f64());
    // Lệnh MCP ghi một dòng mỗi tool ở lớp MCP; request bao ngoài không ghi thêm.
    if let Some(request_log) = &request_log
        && route != super::MCP_PATH
    {
        let span_context = span.context().span().span_context().clone();
        request_log.record(
            &caller,
            Finished {
                method: method.as_str().to_owned(),
                path: route.to_owned(),
                status,
                code: response
                    .extensions()
                    .get::<Arc<Error>>()
                    .map(|error| error.code().to_owned()),
                duration: elapsed,
                idempotency_key,
                approval_id: response
                    .extensions()
                    .get::<crate::ops::approvals::Pending>()
                    .map(|pending| pending.0)
                    .or(approval_header),
                request_id: (!request_id.is_empty()).then(|| request_id.clone()),
                trace_id: span_context.is_valid().then(|| span_context.trace_id().to_string()),
            },
        );
    }
    span.record("http.response.status_code", status);
    span.in_scope(|| match response.extensions().get::<Arc<Error>>() {
        // Lỗi phía máy chủ: ghi cả nguyên nhân gốc — thứ client không được thấy.
        Some(error) if status >= 500 => {
            tracing::error!(status, elapsed_ms, route, error = %error, "request failed");
        }
        _ if status >= 500 => tracing::error!(status, elapsed_ms, route, "request failed"),
        // Health check bị gọi vài giây một lần: ghi ở mức debug để không lấp log.
        _ if health => {
            tracing::debug!(status, elapsed_ms, route, "request completed");
        }
        _ => tracing::info!(status, elapsed_ms, route, "request completed"),
    });
    response
}

/// Span `request` của lớp log, cất trong extensions để phần tìm khách ghi `tenant_id` lên nó (bước 39 B7).
#[derive(Clone)]
pub(crate) struct RequestSpan(pub(crate) tracing::Span);

impl RequestSpan {
    /// Ghi khách lên span của request trong `extensions` (nếu có).
    pub(crate) fn note_tenant(
        extensions: &axum::http::Extensions,
        tenant: crate::tenancy::TenantId,
    ) {
        if let Some(span) = extensions.get::<RequestSpan>() {
            span.0.record("tenant_id", tracing::field::display(tenant));
        }
    }
}
