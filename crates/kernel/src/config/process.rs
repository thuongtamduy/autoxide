//! Cấu hình tiến trình: tiến trình này làm việc gì (API, worker hay cả hai) và chạy bao
//! nhiêu job cùng lúc.

use super::{ConfigReader, Invalid, Setting, parse_in_range};

pub const ROLE: Setting = Setting::new(
    "AUTOXIDE_PROCESS_ROLE",
    "What `autoxide serve` runs: `all` (the API and the background work), `api` (only the \
     API) or `worker` (only background work — jobs, scheduled jobs, event delivery — plus the \
     health checks). Split them to scale the API and the workers separately; run at least one \
     process with `all` or `worker`, or background work never happens.",
)
.with_default("all");

pub const JOB_CONCURRENCY: Setting = Setting::new(
    "AUTOXIDE_JOBS_CONCURRENCY",
    "Jobs one worker process runs at the same time, per module (1 to 256).",
)
.with_default("4");

/// Tiến trình `serve` làm việc gì.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessRole {
    /// API và việc nền.
    All,
    /// Chỉ API.
    Api,
    /// Chỉ việc nền (kèm health check).
    Worker,
}

impl ProcessRole {
    /// Tiến trình phục vụ API của các module.
    pub fn serves_api(self) -> bool {
        matches!(self, ProcessRole::All | ProcessRole::Api)
    }

    /// Tiến trình chạy job, lịch định kỳ và giao event.
    pub fn runs_background_work(self) -> bool {
        matches!(self, ProcessRole::All | ProcessRole::Worker)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessConfig {
    pub role: ProcessRole,
    pub job_concurrency: u64,
}

impl ProcessConfig {
    pub fn read(reader: &mut ConfigReader) -> Result<Self, Invalid> {
        let role = reader.get_with(&ROLE, |raw| match raw {
            "all" => Ok(ProcessRole::All),
            "api" => Ok(ProcessRole::Api),
            "worker" => Ok(ProcessRole::Worker),
            _ => Err("must be `all`, `api` or `worker`".to_owned()),
        });
        let job_concurrency =
            reader.get_with(&JOB_CONCURRENCY, |raw| parse_in_range(raw, 1u64, 256));
        Ok(ProcessConfig { role: role?, job_concurrency: job_concurrency? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<ProcessConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = ProcessConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_lam_ca_hai_viec() {
        let config = read(&[]).unwrap();

        assert_eq!(config, ProcessConfig { role: ProcessRole::All, job_concurrency: 4 });
        assert!(config.role.serves_api() && config.role.runs_background_work());
        let api = read(&[("AUTOXIDE_PROCESS_ROLE", "api")]).unwrap().role;
        assert!(api.serves_api() && !api.runs_background_work());
        let worker = read(&[("AUTOXIDE_PROCESS_ROLE", "worker")]).unwrap().role;
        assert!(!worker.serves_api() && worker.runs_background_work());
        assert!(read(&[("AUTOXIDE_PROCESS_ROLE", "both")]).unwrap_err().contains("must be `all`"));
    }
}
