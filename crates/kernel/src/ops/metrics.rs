//! Metrics theo định dạng Prometheus.
//!
//! Code ở mọi nơi ghi số đo bằng macro của thư viện `metrics`:
//!
//! ```
//! metrics::counter!("notes_created_total").increment(1);
//! ```
//!
//! Tên số đo của module mang tiền tố `<module>_`. Nhãn (label) chỉ nhận giá trị
//! thuộc một tập NHỎ và cố định (method, mẫu route, mã trạng thái): mỗi tổ hợp
//! nhãn là một chuỗi số đo riêng trong bộ nhớ, nên KHÔNG dùng ID, email hay đường
//! dẫn thô làm nhãn.
//!
//! Khi chưa gọi [`Metrics::install`], các macro trên không làm gì.

use std::fmt;
use std::time::Duration;

use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

/// Ranh giới các "xô" đo thời gian (giây), từ 5 ms tới 10 giây.
const DURATION_BUCKETS: [f64; 11] = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0];

/// Lỗi khi bật metrics.
#[derive(Debug)]
pub struct MetricsError(String);

impl fmt::Display for MetricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "metrics: {}", self.0)
    }
}

impl std::error::Error for MetricsError {}

/// Bộ thu số đo của tiến trình. `Clone` rẻ.
#[derive(Clone)]
pub struct Metrics {
    handle: PrometheusHandle,
}

impl Metrics {
    /// Bật thu số đo cho cả tiến trình. Gọi một lần lúc khởi động; lần hai là lỗi.
    pub fn install() -> Result<Self, MetricsError> {
        let handle = PrometheusBuilder::new()
            .set_buckets(&DURATION_BUCKETS)
            .and_then(PrometheusBuilder::install_recorder)
            .map_err(|error| MetricsError(error.to_string()))?;
        Ok(Metrics { handle })
    }

    /// Mọi số đo hiện có, ở định dạng văn bản của Prometheus.
    pub fn render(&self) -> String {
        self.handle.render()
    }

    /// Chạy việc bảo trì định kỳ của bộ thu (gom dữ liệu histogram) ở một task nền,
    /// để bộ nhớ không phình khi không ai đọc `/metrics`. Phải gọi từ trong runtime
    /// tokio.
    pub fn spawn_upkeep(&self) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(5));
            loop {
                ticks.tick().await;
                handle.run_upkeep();
            }
        });
    }
}

impl fmt::Debug for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metrics").finish_non_exhaustive()
    }
}
