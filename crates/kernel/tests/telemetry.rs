//! Xuất trace qua OTLP/HTTP tới một collector. Bộ xuất và bộ đọc `traceparent` là
//! toàn tiến trình, nên file này (một tiến trình test riêng) chỉ có MỘT bài test.

// File trong `tests/` toàn là code test: hỏng thì phải dừng ngay và báo rõ chỗ.
// Clippy chỉ tự miễn cho hàm `#[test]`, không miễn cho hàm phụ, nên khai ở đây.
// Trong `src/` thì KHÔNG được khai dòng này (`cargo xtask check` chặn).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use autoxide_kernel::config::{LogConfig, LogFormat, LogLevel, TraceConfig};
use autoxide_kernel::logging;
use autoxide_kernel::telemetry::{TraceExport, remote_context};
use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use opentelemetry::trace::TraceContextExt;

type Received = Arc<Mutex<Vec<Vec<u8>>>>;

async fn collect(State(received): State<Received>, body: Bytes) {
    received.lock().unwrap().push(body.to_vec());
}

/// Vị trí của `needle` trong `haystack` (dữ liệu nhị phân).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn khong_cau_hinh_collector_thi_khong_xuat_gi() {
    let config = TraceConfig { otlp_endpoint: None, service_name: "x".to_owned() };
    assert!(TraceExport::start(&config).unwrap().is_none());
}

#[test]
fn span_duoc_gui_toi_collector_va_noi_vao_trace_cua_ben_goi() {
    // Bộ xuất trace phải được tạo và tắt NGOÀI runtime tokio, nên bài test này là
    // hàm thường; collector giả chạy trên một runtime riêng ở thread nền.
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let received = Received::default();
    let listener = runtime.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let address = listener.local_addr().unwrap();
    let collector = Router::new()
        .route("/v1/traces", axum::routing::post(collect))
        .with_state(Arc::clone(&received));
    runtime.spawn(async move { axum::serve(listener, collector).await.unwrap() });

    let config = TraceConfig {
        otlp_endpoint: Some(format!("http://{address}")),
        service_name: "autoxide-test".to_owned(),
    };
    let (layer, export) = TraceExport::start(&config).unwrap().unwrap();
    let log = LogConfig { level: LogLevel::Info, format: LogFormat::Text };
    let subscriber = logging::subscriber(&log, std::io::sink, vec![layer]);

    // Request đến mang `traceparent` của bên gọi (chuẩn W3C Trace Context).
    let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
    let mut headers = HeaderMap::new();
    headers.insert(
        "traceparent",
        HeaderValue::from_str(&format!("00-{trace_id}-00f067aa0ba902b7-01")).unwrap(),
    );
    let remote = remote_context(&headers);
    assert_eq!(remote.span().span_context().trace_id().to_string(), trace_id);
    // Request không mang header đó thì không có trace nào để nối vào.
    assert!(!remote_context(&HeaderMap::new()).span().span_context().is_valid());

    tracing::subscriber::with_default(subscriber, || {
        use tracing_opentelemetry::OpenTelemetrySpanExt;
        let span = tracing::info_span!("checkout", "http.route" = "/api/v1/orders");
        span.set_parent(remote).unwrap();
        let _inside = span.enter();
        tracing::info!("inside the span");
    });

    // Tắt = đẩy nốt span còn trong hàng đợi. Sau lời gọi này collector phải nhận đủ.
    export.shutdown().unwrap();

    let bodies = received.lock().unwrap().clone();
    assert!(!bodies.is_empty(), "collector received nothing");
    let all: Vec<u8> = bodies.concat();
    // Thân là protobuf nhị phân; tên span, tên dịch vụ và thuộc tính nằm nguyên
    // dạng chữ bên trong.
    assert!(contains(&all, b"checkout"));
    assert!(contains(&all, b"autoxide-test"));
    assert!(contains(&all, b"/api/v1/orders"));
    // Span mang trace id của bên gọi (16 byte nhị phân).
    let trace_bytes: Vec<u8> = (0..16)
        .map(|index| u8::from_str_radix(&trace_id[index * 2..index * 2 + 2], 16).unwrap())
        .collect();
    assert!(contains(&all, &trace_bytes), "span is not part of the caller's trace");
}
