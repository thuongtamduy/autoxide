//! Bật log và xuất trace cho tiến trình. Mọi lệnh chạm tới hệ thống gọi [`init`]
//! ngay sau khi đọc cấu hình, TRƯỚC khi vào runtime tokio.

use autoxide_host::logging;
use autoxide_host::telemetry::TraceExport;
use autoxide_kernel::config::Config;

/// Tay nắm của phần observability đang chạy. Gọi [`shutdown`](Self::shutdown) trước
/// khi tiến trình thoát (sau khi runtime tokio đã dừng).
pub struct Observability {
    trace_export: Option<TraceExport>,
}

/// Bật xuất trace (nếu được cấu hình) rồi bật log. Gọi một lần, ngoài runtime tokio.
pub fn init(config: &Config) -> Result<Observability, String> {
    let (layers, trace_export) =
        match TraceExport::start(&config.trace).map_err(|error| error.to_string())? {
            Some((layer, export)) => (vec![layer], Some(export)),
            None => (Vec::new(), None),
        };
    logging::init(&config.log, layers).map_err(|error| error.to_string())?;
    Ok(Observability { trace_export })
}

impl Observability {
    /// Đẩy nốt các trace còn trong hàng đợi. Lỗi ở đây không làm lệnh thất bại:
    /// việc chính đã xong, chỉ ghi lại để người vận hành biết.
    pub fn shutdown(self) {
        if let Some(export) = self.trace_export
            && let Err(error) = export.shutdown()
        {
            eprintln!("autoxide: {error}");
        }
    }
}
