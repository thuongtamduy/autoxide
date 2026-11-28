//! Quan sát theo khách (bước 39 B7, D-190): dòng log của request mang `tenant_id` của khách đã
//! xác định, để tra sự cố của một khách bằng log và trace. Chạy trên cả hai database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use autoxide_testkit::{TestApp, TestDatabase};
use tracing::level_filters::LevelFilter;

use super::http::json_subscriber;

/// Nơi ghi log vào bộ nhớ, để test đọc lại.
#[derive(Clone, Default)]
struct LogBuffer(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn log_request_mang_tenant_id() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test).build().await.unwrap();
        let buffer = LogBuffer::default();
        let subscriber = json_subscriber(LevelFilter::INFO, buffer.clone());
        // Xem giải thích ở `http.rs` (`logged`): giữ một bộ nhận rỗng sống suốt process.
        static KEEP_MANY: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
        KEEP_MANY
            .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        let guard = tracing::subscriber::set_default(subscriber);
        let response = app.get("/api/v1/kernel/public/texts").send().await.unwrap();
        drop(guard);
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        let line: serde_json::Value = text
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .find(|line| line["message"] == "request completed")
            .unwrap_or_else(|| panic!("no request line in {text}"));
        let tenant = app.default_tenant().id().to_string();
        assert_eq!(line["span"]["tenant_id"], tenant.as_str(), "{line}");
    })
    .await
    .unwrap();
}
