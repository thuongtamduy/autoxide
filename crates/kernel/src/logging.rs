//! Log của chương trình.
//!
//! Code ở mọi nơi ghi log bằng macro của `tracing` (`tracing::info!(…)`,
//! `tracing::error!(…)`), kèm trường có tên thay vì ghép chuỗi:
//!
//! ```
//! let user_id = 7;
//! tracing::info!(user_id, "password changed");
//! ```
//!
//! File này quyết định log đi đâu và trông thế nào: ra **stderr** (stdout dành cho
//! kết quả của lệnh, vd `autoxide openapi`), dạng chữ cho người đọc hoặc JSON cho
//! máy thu gom, lọc theo mức đã cấu hình.
//!
//! KHÔNG ghi secret, mật khẩu, token hay toàn bộ thân request vào log.

use std::fmt;

use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;

use crate::config::{LogConfig, LogFormat, LogLevel};

/// Lỗi khi bật log.
#[derive(Debug)]
pub struct LogError(String);

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "logging: {}", self.0)
    }
}

impl std::error::Error for LogError {}

/// Một lớp xử lý log đã đóng hộp, để gom nhiều lớp khác kiểu vào một danh sách.
pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// Bật log cho cả tiến trình, ghi ra stderr. Gọi một lần lúc khởi động; gọi lần
/// hai là lỗi. `extra` là các lớp thêm (vd xuất trace ra OpenTelemetry).
pub fn init(config: &LogConfig, extra: Vec<BoxedLayer>) -> Result<(), LogError> {
    tracing::subscriber::set_global_default(subscriber(config, std::io::stderr, extra))
        .map_err(|error| LogError(error.to_string()))
}

/// Dựng bộ nhận log ghi vào `writer`. Tách khỏi [`init`] để test bắt được nội dung
/// log mà không đụng tới cấu hình toàn tiến trình.
pub fn subscriber<W>(
    config: &LogConfig,
    writer: W,
    extra: Vec<BoxedLayer>,
) -> impl Subscriber + Send + Sync
where
    // "Với mọi vòng đời `'a`, `W` tạo được một nơi ghi": đòi hỏi của thư viện để
    // mỗi dòng log mượn nơi ghi trong thời gian ngắn.
    W: for<'a> MakeWriter<'a> + Send + Sync + 'static,
{
    let format: BoxedLayer = match config.format {
        // Không màu: log thường đi vào file hoặc máy thu gom, mã màu chỉ là rác.
        LogFormat::Text => {
            Box::new(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(writer))
        }
        // Mỗi dòng một đối tượng: các trường của sự kiện nằm ở cấp cao nhất, kèm
        // các trường của span hiện tại (vd request id).
        LogFormat::Json => Box::new(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_current_span(true)
                .with_span_list(false)
                .with_writer(writer),
        ),
    };
    let mut layers = vec![format];
    layers.extend(extra);
    // Lọc theo mức áp cho mọi lớp.
    Registry::default().with(layers).with(QuietClientGone).with(level_filter(config.level))
}

/// Thông điệp mà thư viện ghi ở mức ERROR cho một việc bình thường: client ngắt kết nối
/// trước khi nhận xong câu trả lời. Để ở ERROR thì mỗi lần agent huỷ một lời gọi MCP lại
/// làm nhiễu cảnh báo vận hành (đo được khi chạy tải, D-076).
const CLIENT_GONE: [(&str, &str); 1] =
    [("rmcp::service", "failed to send pending response during drain")];

/// Lớp bỏ các thông điệp trong [`CLIENT_GONE`]; mọi log khác đi qua nguyên vẹn.
struct QuietClientGone;

impl<S: Subscriber> Layer<S> for QuietClientGone {
    fn event_enabled(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) -> bool {
        let target = event.metadata().target();
        if !CLIENT_GONE.iter().any(|(gone, _)| target == *gone) {
            return true;
        }
        let mut message = MessageOf(String::new());
        event.record(&mut message);
        !CLIENT_GONE.iter().any(|(gone, text)| target == *gone && message.0 == *text)
    }
}

/// Đọc trường `message` của một sự kiện log.
struct MessageOf(String);

impl tracing::field::Visit for MessageOf {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Trace => LevelFilter::TRACE,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error => LevelFilter::ERROR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Nơi ghi log vào bộ nhớ, để test đọc lại.
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Buffer {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl std::io::Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Buffer;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// Chạy `emit` với bộ nhận log theo `config`; trả về những gì đã được ghi.
    fn capture(config: LogConfig, emit: impl FnOnce()) -> String {
        let buffer = Buffer::default();
        tracing::subscriber::with_default(subscriber(&config, buffer.clone(), Vec::new()), emit);
        buffer.text()
    }

    #[test]
    fn dang_chu_co_muc_noi_ghi_thong_diep_va_truong() {
        let config = LogConfig { level: LogLevel::Info, format: LogFormat::Text };

        let output = capture(config, || tracing::info!(user_id = 7, "password changed"));

        assert!(output.contains(" INFO "), "{output}");
        assert!(output.contains("password changed"), "{output}");
        assert!(output.contains("user_id=7"), "{output}");
        // Không có mã màu.
        assert!(!output.contains('\u{1b}'), "{output:?}");
    }

    #[test]
    fn dang_json_moi_dong_mot_doi_tuong_kem_truong_cua_span() {
        let config = LogConfig { level: LogLevel::Info, format: LogFormat::Json };

        let output = capture(config, || {
            let span = tracing::info_span!("request", request_id = "abc");
            let _inside = span.enter();
            tracing::warn!(attempts = 3, "login throttled");
        });

        let lines: Vec<&str> = output.lines().collect();
        assert_eq!(lines.len(), 1, "{output}");
        let entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry["level"], "WARN");
        assert_eq!(entry["message"], "login throttled");
        assert_eq!(entry["attempts"], 3);
        assert_eq!(entry["span"]["request_id"], "abc");
        assert!(entry["timestamp"].is_string(), "{entry}");
    }

    #[test]
    fn bo_thong_diep_client_ngat_ket_noi_cua_rmcp() {
        let config = LogConfig { level: LogLevel::Info, format: LogFormat::Text };

        let output = capture(config, || {
            tracing::error!(
                target: "rmcp::service",
                error = "channel closed",
                "failed to send pending response during drain"
            );
            tracing::error!(target: "rmcp::service", "fail to close sink");
            tracing::error!("failed to send pending response during drain");
        });

        assert_eq!(output.lines().count(), 2, "{output}");
        assert!(output.contains("fail to close sink"), "{output}");
        assert!(!output.contains("rmcp::service: failed to send pending"), "{output}");
    }

    #[test]
    fn muc_thap_hon_cau_hinh_bi_bo() {
        let config = LogConfig { level: LogLevel::Warn, format: LogFormat::Text };

        let output = capture(config, || {
            tracing::debug!("debug detail");
            tracing::info!("routine");
            tracing::warn!("worth a look");
            tracing::error!("broken");
        });

        assert!(!output.contains("debug detail") && !output.contains("routine"), "{output}");
        assert!(output.contains("worth a look") && output.contains("broken"), "{output}");
    }
}
