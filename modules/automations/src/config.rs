//! Cấu hình của module `automations` (biến môi trường, đọc một lần lúc khởi động).

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const RUN_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_AUTOMATIONS_RUN_RETENTION_DAYS",
    "Days the log of an automation run is kept, 1 to 3660.",
)
.with_default("90");

pub(crate) const MAX_RUNS_PER_HOUR: Setting = Setting::new(
    "AUTOXIDE_AUTOMATIONS_MAX_RUNS_PER_HOUR",
    "Runs one automation rule may start in an hour; past it the rule pauses itself and its author \
     is told, 1 to 100000.",
)
.with_default("1000");

pub(crate) const TICK_SECONDS: Setting = Setting::new(
    "AUTOXIDE_AUTOMATIONS_TICK_SECONDS",
    "Seconds between two checks for automation rules due on a schedule or a metric, 10 to 3600.",
)
.with_default("60");

/// Độ sâu tối đa của một chuỗi luật (luật A làm việc sinh event kích hoạt luật B…).
pub(crate) const MAX_CHAIN_DEPTH: i32 = 3;

/// Cấu hình đã đọc.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AutomationsConfig {
    pub run_retention_days: i64,
    pub max_runs_per_hour: u64,
    pub tick_seconds: u64,
}

impl Default for AutomationsConfig {
    fn default() -> Self {
        AutomationsConfig { run_retention_days: 90, max_runs_per_hour: 1000, tick_seconds: 60 }
    }
}

pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<AutomationsConfig, Invalid> {
    Ok(AutomationsConfig {
        run_retention_days: reader
            .get_with(&RUN_RETENTION_DAYS, |raw| parse_in_range(raw, 1i64, 3660))?,
        max_runs_per_hour: reader
            .get_with(&MAX_RUNS_PER_HOUR, |raw| parse_in_range(raw, 1u64, 100_000))?,
        tick_seconds: reader.get_with(&TICK_SECONDS, |raw| parse_in_range(raw, 10u64, 3600))?,
    })
}
