//! Cấu hình của module `reporting` (biến môi trường, đọc một lần lúc khởi động).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const FORWARD_SECONDS: Setting = Setting::new(
    "AUTOXIDE_REPORTING_FORWARD_SECONDS",
    "Seconds between two runs that push new report facts to the store of the customer tree (the \
     reporting delay), 1 to 3600.",
)
.with_default("5");

pub(crate) const FORWARD_BATCH: Setting = Setting::new(
    "AUTOXIDE_REPORTING_FORWARD_BATCH",
    "Report facts pushed in one transaction at the database of the root tenant, 1 to 5000.",
)
.with_default("500");

pub(crate) const LOCAL_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_REPORTING_LOCAL_RETENTION_DAYS",
    "Days a pushed report fact stays at its own database, so it can be pushed again (rebuild), 1 \
     to 3660.",
)
.with_default("35");

/// Cấu hình đã đọc.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ReportingConfig {
    pub forward_seconds: u64,
    pub forward_batch: u64,
    pub local_retention_days: i64,
}

impl Default for ReportingConfig {
    fn default() -> Self {
        ReportingConfig { forward_seconds: 5, forward_batch: 500, local_retention_days: 35 }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<ReportingConfig, Invalid> {
    Ok(ReportingConfig {
        forward_seconds: reader
            .get_with(&FORWARD_SECONDS, |raw| parse_in_range(raw, 1u64, 3600))?,
        forward_batch: reader.get_with(&FORWARD_BATCH, |raw| parse_in_range(raw, 1u64, 5000))?,
        local_retention_days: reader
            .get_with(&LOCAL_RETENTION_DAYS, |raw| parse_in_range(raw, 1i64, 3660))?,
    })
}
