//! Process configuration: what this process does (API, worker or both) and how many jobs it runs
//! at once.

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

pub const JOB_BLOCKED_DELAY_SECONDS: Setting = Setting::new(
    "AUTOXIDE_JOBS_BLOCKED_DELAY_SECONDS",
    "When a job belongs to a module the tenant's plan no longer includes, or the tenant is \
     read-only, wait this long before looking at it again (60 to 86400 seconds). Such a wait does \
     not count as a failed attempt.",
)
.with_default("3600");

pub const METRICS_BACKLOG_SECONDS: Setting = Setting::new(
    "AUTOXIDE_METRICS_BACKLOG_SECONDS",
    "How often a process that runs background work updates the queue backlog gauges \
     (`jobs_pending`, `jobs_oldest_pending_seconds`, `jobs_dead`, `events_*`, \
     `database_connections`) at `/metrics`, in seconds (5 to 3600). Only used when \
     AUTOXIDE_HTTP_METRICS is on.",
)
.with_default("30");

pub const METRICS_MAX_DATABASE_LABELS: Setting = Setting::new(
    "AUTOXIDE_METRICS_MAX_DATABASE_LABELS",
    "Most databases the backlog gauges name in their `database` label (1 to 10000); the others \
     are added up under `database=\"other\"`, so tenants with their own database cannot make the \
     metrics grow without end.",
)
.with_default("50");

/// What the `serve` process does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProcessRole {
    /// API and background work.
    All,
    /// API only.
    Api,
    /// Background work only (plus health check).
    Worker,
}

impl ProcessRole {
    /// The process serves the modules' API.
    pub fn serves_api(self) -> bool {
        matches!(self, ProcessRole::All | ProcessRole::Api)
    }

    /// The process runs jobs, periodic schedules and event delivery.
    pub fn runs_background_work(self) -> bool {
        matches!(self, ProcessRole::All | ProcessRole::Worker)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProcessConfig {
    pub role: ProcessRole,
    pub job_concurrency: u64,
    /// Delay for a job blocked by a package, in seconds (step 35).
    pub job_blocked_delay: u64,
    /// Refresh interval of the backlog gauge, in seconds (step 39).
    pub metrics_backlog_seconds: u64,
    /// Maximum number of `database` labels on the backlog gauge.
    pub metrics_max_database_labels: u64,
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
        let job_blocked_delay =
            reader.get_with(&JOB_BLOCKED_DELAY_SECONDS, |raw| parse_in_range(raw, 60u64, 86_400));
        let metrics_backlog_seconds =
            reader.get_with(&METRICS_BACKLOG_SECONDS, |raw| parse_in_range(raw, 5u64, 3600));
        let metrics_max_database_labels =
            reader.get_with(&METRICS_MAX_DATABASE_LABELS, |raw| parse_in_range(raw, 1u64, 10_000));
        Ok(ProcessConfig {
            role: role?,
            job_concurrency: job_concurrency?,
            job_blocked_delay: job_blocked_delay?,
            metrics_backlog_seconds: metrics_backlog_seconds?,
            metrics_max_database_labels: metrics_max_database_labels?,
        })
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

        assert_eq!(
            config,
            ProcessConfig {
                role: ProcessRole::All,
                job_concurrency: 4,
                job_blocked_delay: 3600,
                metrics_backlog_seconds: 30,
                metrics_max_database_labels: 50,
            }
        );
        assert!(config.role.serves_api() && config.role.runs_background_work());
        let api = read(&[("AUTOXIDE_PROCESS_ROLE", "api")]).unwrap().role;
        assert!(api.serves_api() && !api.runs_background_work());
        let worker = read(&[("AUTOXIDE_PROCESS_ROLE", "worker")]).unwrap().role;
        assert!(!worker.serves_api() && worker.runs_background_work());
        assert!(read(&[("AUTOXIDE_PROCESS_ROLE", "both")]).unwrap_err().contains("must be `all`"));
    }
}
