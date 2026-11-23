//! Multi-tenant configuration, prefix `AUTOXIDE_TENANCY_`. The default is `single` mode: someone installing
//! one store has nothing to set.

use std::path::PathBuf;

use crate::config::{ConfigReader, Invalid, Setting, parse_in_range};

use super::{MULTI_TENANT_READY, TenancyMode};

const MODE: Setting = Setting::new(
    "AUTOXIDE_TENANCY_MODE",
    "`single` (one customer: the default, nothing else to set) or `multi` (many customers, told \
     apart by domain, credentials or the `Autoxide-Tenant` header; needs `AUTOXIDE_SECRET_KEY`).",
)
.with_default("single");

const CACHE_SECONDS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_CACHE_SECONDS",
    "Seconds a looked-up tenant is cached (1 to 3600) in `multi` mode. Suspending a tenant takes \
     effect on other servers within this time.",
)
.with_default("10");

const MAX_OPEN_DATABASES: Setting = Setting::new(
    "AUTOXIDE_TENANCY_MAX_OPEN_DATABASES",
    "Most tenant databases (placements) kept open at once (1 to 10000); the least used one is \
     closed when more are needed.",
)
.with_default("32");

const WRITE_CONNECTIONS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_WRITE_CONNECTIONS",
    "Write connections of each tenant database other than DATABASE_URL (1 to 100). SQLite always \
     uses one.",
)
.with_default("2");

const READ_CONNECTIONS: Setting = Setting::new(
    "AUTOXIDE_TENANCY_READ_CONNECTIONS",
    "Read connections of each tenant database other than DATABASE_URL (1 to 100).",
)
.with_default("4");

const SQLITE_DIR: Setting = Setting::new(
    "AUTOXIDE_TENANCY_SQLITE_DIR",
    "Directory of the SQLite file of each tenant with its own database. Default: `tenants/` next \
     to the file of DATABASE_URL.",
)
.with_example("data/tenants");

const WORKER_PLACEMENTS: Setting = Setting::new(
    "AUTOXIDE_WORKER_PLACEMENTS",
    "Placements whose jobs and events this server runs: `all`, or placement names separated by \
     commas.",
)
.with_default("all");

/// Which placements this server runs jobs and events for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorkerPlacements {
    #[default]
    All,
    /// By placement name.
    Named(Vec<String>),
}

impl WorkerPlacements {
    /// Whether this server runs the jobs of the placement named `name`.
    pub fn includes(&self, name: &str) -> bool {
        match self {
            WorkerPlacements::All => true,
            WorkerPlacements::Named(names) => names.iter().any(|known| known == name),
        }
    }
}

/// Multi-tenant configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TenancyConfig {
    pub mode: TenancyMode,
    pub cache_seconds: u64,
    pub max_open_databases: usize,
    pub write_connections: u32,
    pub read_connections: u32,
    /// `None`: `tenants/` next to the `DATABASE_URL` file.
    pub sqlite_dir: Option<PathBuf>,
    pub worker_placements: WorkerPlacements,
}

impl Default for TenancyConfig {
    fn default() -> Self {
        TenancyConfig {
            mode: TenancyMode::Single,
            cache_seconds: 10,
            max_open_databases: 32,
            write_connections: 2,
            read_connections: 4,
            sqlite_dir: None,
            worker_placements: WorkerPlacements::All,
        }
    }
}

impl TenancyConfig {
    /// Default configuration in mode `mode` (for testkit and tests, ignoring `MULTI_TENANT_READY`).
    pub fn with_mode(mode: TenancyMode) -> Self {
        TenancyConfig { mode, ..TenancyConfig::default() }
    }

    /// Sets the maximum number of placement pools open at once.
    pub fn with_max_open_databases(mut self, max: usize) -> Self {
        self.max_open_databases = max;
        self
    }
}

/// Reads the multi-tenant configuration (`multi` mode additionally needs `AUTOXIDE_SECRET_KEY`, checked in the `kernel` module).
pub(crate) fn read_setting(reader: &mut ConfigReader<'_>) -> Result<TenancyConfig, Invalid> {
    let mode = reader.get_with(&MODE, |raw| match raw.trim() {
        "single" => Ok(TenancyMode::Single),
        "multi" if MULTI_TENANT_READY => Ok(TenancyMode::Multi),
        "multi" => Err("`multi` is not available yet in this version: not every module keeps \
                        its data per tenant. Use `single`"
            .to_owned()),
        _ => Err("must be `single` or `multi`".to_owned()),
    });
    let cache = reader.get_with(&CACHE_SECONDS, |raw| parse_in_range(raw, 1u64, 3600));
    let open = reader.get_with(&MAX_OPEN_DATABASES, |raw| parse_in_range(raw, 1usize, 10_000));
    let write = reader.get_with(&WRITE_CONNECTIONS, |raw| parse_in_range(raw, 1u32, 100));
    let read = reader.get_with(&READ_CONNECTIONS, |raw| parse_in_range(raw, 1u32, 100));
    let sqlite_dir = reader.optional_with(&SQLITE_DIR, |raw| {
        let raw = raw.trim();
        if raw.is_empty() { Err("must not be empty".to_owned()) } else { Ok(PathBuf::from(raw)) }
    });
    let workers = reader.get_with(&WORKER_PLACEMENTS, |raw| {
        let raw = raw.trim();
        if raw == "all" {
            return Ok(WorkerPlacements::All);
        }
        let names: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect();
        if names.is_empty() {
            Err("must be `all` or placement names separated by commas".to_owned())
        } else {
            Ok(WorkerPlacements::Named(names))
        }
    });
    Ok(TenancyConfig {
        mode: mode?,
        cache_seconds: cache?,
        max_open_databases: open?,
        write_connections: write?,
        read_connections: read?,
        sqlite_dir: sqlite_dir?,
        worker_placements: workers?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<TenancyConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = read_setting(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_la_single_khong_can_dat_gi() {
        assert_eq!(read(&[]).unwrap(), TenancyConfig::default());
    }

    #[test]
    fn multi_bi_khoa_khi_chua_san_sang_va_gia_tri_sai_bao_cung_luc() {
        let error = read(&[
            ("AUTOXIDE_TENANCY_MODE", "multi"),
            ("AUTOXIDE_TENANCY_MAX_OPEN_DATABASES", "0"),
        ])
        .unwrap_err();
        if !MULTI_TENANT_READY {
            assert!(
                error.contains("AUTOXIDE_TENANCY_MODE") && error.contains("not available"),
                "{error}"
            );
        }
        assert!(error.contains("MAX_OPEN_DATABASES"), "{error}");
        let config = read(&[("AUTOXIDE_WORKER_PLACEMENTS", "main, eu ")]).unwrap();
        assert!(config.worker_placements.includes("eu"));
        assert!(!config.worker_placements.includes("us"));
    }
}
