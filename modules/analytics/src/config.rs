//! Cấu hình của module `analytics`, tiền tố `AUTOXIDE_ANALYTICS_`. Mọi biến có mặc định chạy được
//! ngay. Hạn lưu là cấu hình (không cài cứng): người cài tự đối chiếu văn bản gốc cho trường hợp
//! của mình.

use autoxide_kernel::config::{ConfigReader, Invalid, Setting, parse_in_range};

pub(crate) const ENABLED: Setting = Setting::new(
    "AUTOXIDE_ANALYTICS_ENABLED",
    "Take visit events from websites (`true` or `false`). Off: the public endpoint accepts and \
     drops everything; reports keep what was collected.",
)
.with_default("true");

pub(crate) const RAW_RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS",
    "Days single visit events are kept before a daily job deletes them (2 to 90). Reports read \
     the daily totals, which stay longer.",
)
.with_default("7");

pub(crate) const RETENTION_DAYS: Setting = Setting::new(
    "AUTOXIDE_ANALYTICS_RETENTION_DAYS",
    "Days the daily totals of visits are kept (30 to 3650). The default, 790, is two years and \
     two months, to compare a month with the same month last year.",
)
.with_default("790");

pub(crate) const EVENTS_PER_MINUTE: Setting = Setting::new(
    "AUTOXIDE_ANALYTICS_EVENTS_PER_MINUTE",
    "Visit events taken per minute from one network block (IPv4 /24, IPv6 /64) (1 to 100000); \
     more are dropped silently.",
)
.with_default("120");

pub(crate) const ALLOWED_HOSTS: Setting = Setting::new(
    "AUTOXIDE_ANALYTICS_ALLOWED_HOSTS",
    "Website hosts, comma-separated (e.g. `shop.example,www.shop.example`), whose pages may send \
     visit events besides the host the API is called on and the verified domains of the tenant. \
     Events from other pages are dropped.",
)
.with_default("");

/// Host website gửi sự kiện theo khách (mục 6 của đợt cấu hình theo khách); tên miền đã xác minh của khách
/// luôn được nhận, không cần khai.
pub(crate) const TENANT_ALLOWED_HOSTS: autoxide_kernel::tenancy::TenantSetting =
    autoxide_kernel::tenancy::TenantSetting::new(
        "analytics.allowed_hosts",
        "Website hosts, comma-separated (e.g. `shop.example,www.shop.example`, at most 50), whose \
         pages may send this tenant's visit events, besides the host the API is called on and this \
         tenant's verified domains (always accepted), instead of the server's \
         AUTOXIDE_ANALYTICS_ALLOWED_HOSTS. Events from other pages are dropped.",
    )
    .checked(|raw| tenant_allowed_hosts(raw).map(|_| ()));

/// Danh sách host (`analytics.allowed_hosts`): tên miền hợp lệ, chữ thường, không trùng, tối đa 50.
pub(crate) fn tenant_allowed_hosts(
    raw: &str,
) -> Result<Vec<String>, autoxide_kernel::error::FieldError> {
    let mut hosts = Vec::new();
    let mut bad = Vec::new();
    for item in raw.split(',').map(str::trim).filter(|item| !item.is_empty()) {
        match autoxide_kernel::tenancy::normalize_domain(item) {
            Some(host) if !hosts.contains(&host) => hosts.push(host),
            Some(_) => {}
            None => bad.push(item.to_owned()),
        }
    }
    if !bad.is_empty() || hosts.is_empty() || hosts.len() > 50 {
        return Err(autoxide_kernel::error::FieldError::new(
            "value",
            "analytics.allowed_hosts_invalid",
            "must be 1 to 50 host names separated by commas, such as `shop.example`; not valid: \
             {items}",
        )
        .with_param("items", bad.join(", ")));
    }
    Ok(hosts)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnalyticsConfig {
    pub enabled: bool,
    pub raw_retention_days: u32,
    pub retention_days: u32,
    pub events_per_minute: u32,
    /// Chữ thường, không cổng.
    pub allowed_hosts: Vec<String>,
}

impl Default for AnalyticsConfig {
    fn default() -> Self {
        AnalyticsConfig {
            enabled: true,
            raw_retention_days: 7,
            retention_days: 790,
            events_per_minute: 120,
            allowed_hosts: Vec::new(),
        }
    }
}

impl AnalyticsConfig {
    pub(crate) fn read(reader: &mut ConfigReader<'_>) -> Result<Self, Invalid> {
        // Đọc hết rồi mới `?`: mọi giá trị sai được báo cùng lúc.
        let enabled = reader.get_with(&ENABLED, |raw| match raw.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("must be `true` or `false`".to_owned()),
        });
        let raw_days = reader.get_with(&RAW_RETENTION_DAYS, |raw| parse_in_range(raw, 2u32, 90));
        let days = reader.get_with(&RETENTION_DAYS, |raw| parse_in_range(raw, 30u32, 3650));
        let per_minute =
            reader.get_with(&EVENTS_PER_MINUTE, |raw| parse_in_range(raw, 1u32, 100_000));
        let hosts = reader.get_with(&ALLOWED_HOSTS, |raw| {
            Ok::<_, String>(
                raw.split(',')
                    .map(|host| host.trim().trim_end_matches('.').to_ascii_lowercase())
                    .filter(|host| !host.is_empty())
                    .collect::<Vec<_>>(),
            )
        });
        Ok(AnalyticsConfig {
            enabled: enabled?,
            raw_retention_days: raw_days?,
            retention_days: days?,
            events_per_minute: per_minute?,
            allowed_hosts: hosts?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autoxide_kernel::config::ConfigSource;

    fn read(pairs: &[(&str, &str)]) -> Result<AnalyticsConfig, String> {
        let source = ConfigSource::from_pairs(pairs.iter().copied());
        let mut reader = ConfigReader::new(&source);
        let config = AnalyticsConfig::read(&mut reader);
        reader.finish(config).map(|(config, _)| config).map_err(|errors| errors.to_string())
    }

    #[test]
    fn mac_dinh_chay_duoc_va_giong_default() {
        assert_eq!(read(&[]).unwrap(), AnalyticsConfig::default());
    }

    #[test]
    fn gia_tri_sai_bi_bao_cung_luc() {
        let error = read(&[
            ("AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS", "1"),
            ("AUTOXIDE_ANALYTICS_ENABLED", "yes"),
        ])
        .unwrap_err();
        assert!(error.contains("RAW_RETENTION_DAYS") && error.contains("ENABLED"), "{error}");
        let config =
            read(&[("AUTOXIDE_ANALYTICS_ALLOWED_HOSTS", " Shop.Example., b.example ")]).unwrap();
        assert_eq!(config.allowed_hosts, ["shop.example", "b.example"]);
    }
}
