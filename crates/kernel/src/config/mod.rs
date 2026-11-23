//! Configuration: read from environment variables (and the `.env` file), validated at startup.
//!
//! - Each variable is declared once with a [`Setting`], next to the code that uses it.
//! - [`ConfigSource`] collects raw values; [`ConfigReader`] converts types, validates, and collects every
//!   error to report them in one pass.
//! - [`Config`] is the kernel configuration. Modules read their own configuration with the same
//!   [`ConfigReader`], so kernel and module errors show up together in one message.
//!
//! Variable naming convention: the industry-standard name if one exists (`DATABASE_URL`), otherwise the
//! `AUTOXIDE_` prefix (docs/DECISIONS.md D-034).

mod database;
mod env_example;
mod http;
mod log;
mod process;
mod reader;
mod setting;
mod source;
mod trace;

pub use database::{DatabaseBackend, DatabaseConfig, DatabaseUrl, redact_password};
pub use env_example::render_env_example;
pub use http::{CorsOrigins, HttpConfig, IpRange, RateLimit, RateLimitStore, SecurityHeaders};
pub use log::{LogConfig, LogFormat, LogLevel};
pub use process::{ProcessConfig, ProcessRole};
pub use reader::{
    ConfigError, ConfigErrors, ConfigReader, Invalid, Origin, SettingValue, parse_in_range,
};
pub use setting::Setting;
pub use source::ConfigSource;
pub use trace::TraceConfig;

/// The kernel configuration. Each roadmap item adds its own group here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    pub database: DatabaseConfig,
    pub http: HttpConfig,
    pub log: LogConfig,
    pub trace: TraceConfig,
    pub process: ProcessConfig,
    /// Multi-tenancy (`AUTOXIDE_TENANCY_*`, step 33).
    pub tenancy: crate::tenancy::TenancyConfig,
}

impl Config {
    /// Reads the kernel configuration. Errors are recorded in `reader`; call [`ConfigReader::finish`] after
    /// every part (kernel and modules) has been read.
    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        let database = DatabaseConfig::read(reader);
        let http = HttpConfig::read(reader);
        let log = LogConfig::read(reader);
        let trace = TraceConfig::read(reader);
        let process = ProcessConfig::read(reader);
        let tenancy = crate::tenancy::read_setting(reader);
        Ok(Config {
            database: database?,
            http: http?,
            log: log?,
            trace: trace?,
            process: process?,
            tenancy: tenancy?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn khong_dat_gi_thi_cau_hinh_mac_dinh_hop_le() {
        // Người lạ clone về chạy ngay được: mọi biến đều có mặc định dùng được.
        let source = ConfigSource::default();
        let mut reader = ConfigReader::new(&source);
        let config = Config::read(&mut reader);

        let (config, values) = reader.finish(config).unwrap();

        assert_eq!(config.database.url.backend(), DatabaseBackend::Sqlite);
        assert!(values.iter().all(|value| value.origin != Origin::Configured));
    }
}
