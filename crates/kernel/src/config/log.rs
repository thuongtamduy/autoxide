//! Cấu hình log.

use super::{ConfigReader, Invalid, Setting};

pub const LEVEL: Setting = Setting::new(
    "AUTOXIDE_LOG_LEVEL",
    "Least severe level to log: `trace`, `debug`, `info`, `warn` or `error`.",
)
.with_default("info");

pub const FORMAT: Setting = Setting::new(
    "AUTOXIDE_LOG_FORMAT",
    "Log format: `text` for people reading a terminal, `json` (one object per line) for log \
     collectors.",
)
.with_default("text");

/// Mức log, từ chi tiết nhất tới nghiêm trọng nhất.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Dòng chữ cho người đọc.
    Text,
    /// Mỗi dòng một đối tượng JSON, cho máy thu gom log.
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogConfig {
    pub level: LogLevel,
    pub format: LogFormat,
}

impl LogConfig {
    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        let level = reader.get_with(&LEVEL, |raw| match raw {
            "trace" => Ok(LogLevel::Trace),
            "debug" => Ok(LogLevel::Debug),
            "info" => Ok(LogLevel::Info),
            "warn" => Ok(LogLevel::Warn),
            "error" => Ok(LogLevel::Error),
            _ => Err("must be `trace`, `debug`, `info`, `warn` or `error`".to_owned()),
        });
        let format = reader.get_with(&FORMAT, |raw| match raw {
            "text" => Ok(LogFormat::Text),
            "json" => Ok(LogFormat::Json),
            _ => Err("must be `text` or `json`".to_owned()),
        });
        Ok(LogConfig { level: level?, format: format? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<LogConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = LogConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_la_info_dang_chu() {
        assert_eq!(
            read(&[]).unwrap(),
            LogConfig { level: LogLevel::Info, format: LogFormat::Text }
        );
    }

    #[test]
    fn nhan_moi_muc_va_dinh_dang() {
        for (raw, level) in [
            ("trace", LogLevel::Trace),
            ("debug", LogLevel::Debug),
            ("info", LogLevel::Info),
            ("warn", LogLevel::Warn),
            ("error", LogLevel::Error),
        ] {
            assert_eq!(read(&[("AUTOXIDE_LOG_LEVEL", raw)]).unwrap().level, level);
        }
        assert_eq!(read(&[("AUTOXIDE_LOG_FORMAT", "json")]).unwrap().format, LogFormat::Json);
    }

    #[test]
    fn gia_tri_la_bi_tu_choi() {
        let message =
            read(&[("AUTOXIDE_LOG_LEVEL", "verbose"), ("AUTOXIDE_LOG_FORMAT", "xml")]).unwrap_err();
        assert!(message.contains("AUTOXIDE_LOG_LEVEL: must be `trace`, `debug`"), "{message}");
        assert!(message.contains("AUTOXIDE_LOG_FORMAT: must be `text` or `json`"), "{message}");
    }
}
