//! The program's logs.
//!
//! Code everywhere logs with the `tracing` macros (`tracing::info!(…)`,
//! `tracing::error!(…)`), with named fields instead of string concatenation:
//!
//! ```
//! let user_id = 7;
//! tracing::info!(user_id, "password changed");
//! ```
//!
//! This file decides where logs go and what they look like: to **stderr** (stdout is reserved for
//! command output, e.g. `autoxide openapi`), as text for people or JSON for
//! collectors, filtered by the configured level.
//!
//! Do NOT log secrets, passwords, tokens or whole request bodies.

use std::fmt;

use tracing::Subscriber;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::Registry;

use autoxide_kernel::config::{LogConfig, LogFormat, LogLevel};

/// Error when enabling logging.
#[derive(Debug)]
pub struct LogError(String);

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "logging: {}", self.0)
    }
}

impl std::error::Error for LogError {}

/// A boxed log-processing layer, so layers of different types can be collected into one list.
pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

/// Enables logging for the whole process, writing to stderr. Call once at startup; a second call
/// is an error. `extra` holds additional layers (e.g. exporting traces to OpenTelemetry).
pub fn init(config: &LogConfig, extra: Vec<BoxedLayer>) -> Result<(), LogError> {
    tracing::subscriber::set_global_default(subscriber(config, std::io::stderr, extra))
        .map_err(|error| LogError(error.to_string()))
}

/// Builds a log subscriber that writes to `writer`. Split from [`init`] so tests can capture the log content
/// without touching process-wide configuration.
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
        // `LogFormat::Text`, và dạng kernel thêm sau này (enum `#[non_exhaustive]`) tới khi crate này biết nó.
        // Không màu: log thường đi vào file hoặc máy thu gom, mã màu chỉ là rác.
        _ => Box::new(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(writer)),
    };
    let mut layers = vec![format];
    layers.extend(extra);
    // Lọc theo mức áp cho mọi lớp.
    Registry::default().with(layers).with(level_filter(config.level))
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

    /// A place that writes logs into memory, so tests can read them back.
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

    /// Runs `emit` with a log subscriber built from `config`; returns what was written.
    fn capture(config: LogConfig, emit: impl FnOnce()) -> String {
        let buffer = Buffer::default();
        tracing::subscriber::with_default(subscriber(&config, buffer.clone(), Vec::new()), emit);
        buffer.text()
    }

    #[test]
    fn dang_chu_co_muc_noi_ghi_thong_diep_va_truong() {
        let config = LogConfig::new(LogLevel::Info, LogFormat::Text);

        let output = capture(config, || tracing::info!(user_id = 7, "password changed"));

        assert!(output.contains(" INFO "), "{output}");
        assert!(output.contains("password changed"), "{output}");
        assert!(output.contains("user_id=7"), "{output}");
        // Không có mã màu.
        assert!(!output.contains('\u{1b}'), "{output:?}");
    }

    #[test]
    fn dang_json_moi_dong_mot_doi_tuong_kem_truong_cua_span() {
        let config = LogConfig::new(LogLevel::Info, LogFormat::Json);

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
    fn muc_thap_hon_cau_hinh_bi_bo() {
        let config = LogConfig::new(LogLevel::Warn, LogFormat::Text);

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
